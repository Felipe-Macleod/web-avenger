//! Integração com o itch.io (Configurações → Apps conectados).
//!
//! A Biblioteca chama `avengerItch(ação, callback)`; o processo de renderização repassa
//! por mensagem ao processo principal, que tem o token e faz as requisições.

pub mod oauth;
pub mod sync;
pub mod token_store;

use crate::http;
use cef::*;
use serde_json::{json, Value};

/// Renderização → principal: `[id, ação]` (`status`, `connect`, `sync`, `disconnect`).
pub const REQUEST_MESSAGE: &str = "avenger.itch.request";
/// Principal → renderização: `[id, json]`.
pub const RESPONSE_MESSAGE: &str = "avenger.itch.response";

pub fn handle_request(frame: &mut Frame, message: &ProcessMessage) -> bool {
    if CefStringUtf16::from(&message.name()).to_string() != REQUEST_MESSAGE {
        return false;
    }
    let Some(args) = message.argument_list() else {
        return true;
    };
    let id = args.int(0);
    let action = CefStringUtf16::from(&args.string(1)).to_string();
    let frame = frame.clone();
    let reply = move |value: Value| respond(&frame, id, &value);

    match action.as_str() {
        "status" => reply(status()),
        // Devolve a URL de autorização; a Biblioteca navega até ela.
        "connect" => reply(connect()),
        "sync" => sync(Box::new(reply)),
        "disconnect" => disconnect(Box::new(reply)),
        other => reply(json!({ "ok": false, "error": format!("ação desconhecida: {other}") })),
    }
    true
}

fn respond(frame: &Frame, id: i32, value: &Value) {
    let Some(mut message) = process_message_create(Some(&CefString::from(RESPONSE_MESSAGE))) else {
        return;
    };
    if let Some(args) = message.argument_list() {
        args.set_int(0, id);
        args.set_string(1, Some(&CefString::from(value.to_string().as_str())));
    }
    frame.send_process_message(ProcessId::RENDERER, Some(&mut message));
}

fn status() -> Value {
    let has_client_id = oauth::client_id().is_some();
    match token_store::load() {
        Ok(Some(token)) => {
            // Conexões feitas antes de um escopo novo (ex.: `collection:view`) precisam
            // autorizar de novo; o itch.io devolve o mesmo token com os escopos somados.
            let missing: Vec<&str> = oauth::SCOPES
                .split_whitespace()
                .filter(|scope| !token.scopes.iter().any(|granted| granted == scope))
                .collect();
            json!({
                "client_id": has_client_id,
                "connected": true,
                "user": token.user,
                "scopes": token.scopes,
                "missing_scopes": missing,
            })
        }
        Ok(None) => json!({ "client_id": has_client_id, "connected": false }),
        Err(error) => json!({ "client_id": has_client_id, "connected": false, "error": error }),
    }
}

fn connect() -> Value {
    let Some(client_id) = oauth::client_id() else {
        return json!({ "ok": false, "error": "Defina ITCH_CLIENT_ID (veja o README)." });
    };
    // Sem chaveiro não há onde guardar a chave do token: falha antes do login.
    if let Err(error) = token_store::ensure_key() {
        return json!({ "ok": false, "error": error });
    }
    match oauth::begin(&client_id) {
        Ok(url) => json!({ "ok": true, "url": url }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

fn sync(reply: Box<dyn FnOnce(Value)>) {
    let token = match token_store::load() {
        Ok(Some(token)) => token.access_token,
        Ok(None) => return reply(json!({ "ok": false, "error": "Conecte ao itch.io primeiro." })),
        Err(error) => return reply(json!({ "ok": false, "error": error })),
    };
    sync::run(
        token,
        Box::new(move |result| match result {
            Ok(report) => reply(json!({ "ok": true, "report": report })),
            Err(error) => reply(json!({ "ok": false, "error": error })),
        }),
    );
}

fn disconnect(reply: Box<dyn FnOnce(Value)>) {
    let token = token_store::load().ok().flatten();
    let finish = move || match token_store::clear() {
        Ok(()) => reply(json!({ "ok": true })),
        Err(error) => reply(json!({ "ok": false, "error": error })),
    };
    match (token, oauth::client_id()) {
        (Some(token), Some(client_id)) => oauth::revoke(&client_id, &token.access_token, Box::new(finish)),
        _ => finish(),
    }
}

/// Chamado em toda navegação (`RequestHandler::on_before_browse`). Se for o retorno do
/// OAuth, conclui o login, volta para a Biblioteca e retorna `true` (cancelar a navegação).
pub fn handle_navigation(frame: &mut Frame, url: &str) -> bool {
    let Some(redirect) = oauth::parse_redirect(url) else {
        return false;
    };
    let frame = frame.clone();
    let back = move |result: Result<String, String>| {
        let hash = match result {
            Ok(user) => format!("itch=ok&user={}", http::url_encode(&user)),
            Err(error) => format!("itch=erro&msg={}", http::url_encode(&error)),
        };
        let url = format!("{}#{hash}", crate::library_url());
        frame.load_url(Some(&CefString::from(url.as_str())));
    };

    let (code, verifier) = match redirect {
        oauth::Redirect::Error(error) => {
            back(Err(error));
            return true;
        }
        oauth::Redirect::Code { code, state } => match oauth::take_verifier(&state) {
            Ok(verifier) => (code, verifier),
            Err(error) => {
                back(Err(error));
                return true;
            }
        },
    };
    let Some(client_id) = oauth::client_id() else {
        back(Err("ITCH_CLIENT_ID não definido.".into()));
        return true;
    };

    oauth::exchange(
        &client_id,
        &code,
        &verifier,
        Box::new(move |result| {
            let (token, scopes) = match result {
                Ok(token) => token,
                Err(error) => return back(Err(error)),
            };
            // Nome do usuário para "Conectado como …".
            let profile_token = token.clone();
            http::fetch(
                http::Request::get("https://api.itch.io/profile").bearer(&profile_token),
                Box::new(move |result| {
                    let user = result
                        .ok()
                        .filter(|response| response.ok())
                        .and_then(|response| serde_json::from_slice::<Value>(&response.body).ok())
                        .and_then(|json| serde_json::from_value::<token_store::ItchUser>(json["user"].clone()).ok());
                    let name = user
                        .as_ref()
                        .map(|u| u.display_name.clone().filter(|n| !n.is_empty()).unwrap_or(u.username.clone()))
                        .unwrap_or_default();
                    let data = token_store::TokenData { access_token: token, scopes, user };
                    back(token_store::save(&data).map(|()| name));
                }),
            );
        }),
    );
    true
}
