//! Integração com o DMM GAMES e o FANZA GAMES (Configurações → Apps conectados).
//!
//! O DMM não tem OAuth: "Conectar" leva aos logins do próprio site (o do DMM e depois o
//! do FANZA, que tem sessão separada) dentro do app, no perfil principal, e a sessão
//! fica nos cookies do perfil. A Biblioteca chama `avengerDmm(ação, callback)`; o
//! processo principal responde, como em `crate::itch`.

pub mod sync;

use crate::http;
use cef::*;
use serde_json::{json, Value};
use std::{cell::RefCell, rc::Rc};

/// Renderização → principal: `[id, ação]` (`status`, `connect`, `sync`, `disconnect`).
pub const REQUEST_MESSAGE: &str = "avenger.dmm.request";
/// Principal → renderização: `[id, json]`.
pub const RESPONSE_MESSAGE: &str = "avenger.dmm.response";

/// Cookie de sessão de longa duração, nos dois domínios.
const SESSION_COOKIE: &str = "login_secure_id";
const GENERAL_SITE: &str = "https://games.dmm.com/";
const ADULT_SITE: &str = "https://games.dmm.co.jp/";

/// Para onde os logins voltam (`/=/path=`); a navegação é interceptada em `handle_navigation`.
const GENERAL_DONE: &str = "https://library.games.dmm.com/?web-avenger=dmm-ok";
const ADULT_DONE: &str = "https://games.dmm.co.jp/?web-avenger=fanza-ok";

fn login_url(accounts_host: &str, back: &str) -> String {
    format!("https://{accounts_host}/service/login/password/=/path={}", http::url_encode(back))
}

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
        "status" => status(Box::new(reply)),
        // Devolve a URL de login; a Biblioteca navega até ela.
        "connect" => reply(json!({ "ok": true, "url": login_url("accounts.dmm.com", GENERAL_DONE) })),
        "sync" => sync::run(Box::new(move |result| match result {
            Ok((report, user)) => reply(json!({ "ok": true, "report": report, "user": user })),
            Err(error) => reply(json!({ "ok": false, "error": error })),
        })),
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

/// Conectado = há sessão no DMM; `adult` = também no FANZA.
fn status(reply: Box<dyn FnOnce(Value)>) {
    has_session(
        GENERAL_SITE,
        Box::new(move |general| {
            has_session(
                ADULT_SITE,
                Box::new(move |adult| reply(json!({ "connected": general, "adult": adult }))),
            )
        }),
    );
}

/// Se o perfil principal tem o cookie de sessão para `url`.
fn has_session(url: &str, done: Box<dyn FnOnce(bool)>) {
    let Some(manager) = cookie_manager_get_global_manager(None) else {
        return done(false);
    };
    let found = Rc::new(VisitResult { found: RefCell::new(false), done: RefCell::new(Some(done)) });
    let mut visitor = SessionVisitor::new(found);
    // Sem cookies, `visit` nunca é chamado: a resposta sai quando o visitor é liberado.
    manager.visit_url_cookies(Some(&CefString::from(url)), 1, Some(&mut visitor));
}

struct VisitResult {
    found: RefCell<bool>,
    done: RefCell<Option<Box<dyn FnOnce(bool)>>>,
}

impl Drop for VisitResult {
    fn drop(&mut self) {
        if let Some(done) = self.done.borrow_mut().take() {
            done(*self.found.borrow());
        }
    }
}

wrap_cookie_visitor! {
    struct SessionVisitor {
        result: Rc<VisitResult>,
    }

    impl CookieVisitor {
        fn visit(
            &self,
            cookie: Option<&Cookie>,
            _count: ::std::os::raw::c_int,
            _total: ::std::os::raw::c_int,
            _delete_cookie: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let found = cookie.is_some_and(|cookie| {
                cookie.name.to_string() == SESSION_COOKIE
                    && !cookie.value.to_string().is_empty()
            });
            if found {
                *self.result.found.borrow_mut() = true;
                return 0; // achou: para a visita
            }
            1
        }
    }
}

/// Apaga os cookies do DMM e do FANZA (sessão, verificação de idade...).
fn disconnect(reply: Box<dyn FnOnce(Value)>) {
    let Some(manager) = cookie_manager_get_global_manager(None) else {
        return reply(json!({ "ok": false, "error": "cookies indisponíveis" }));
    };
    // Cada URL apaga os cookies que valem para ela, inclusive os de domínio (`.dmm.com`).
    let urls = ["https://www.dmm.com/", "https://accounts.dmm.com/", "https://www.dmm.co.jp/", "https://accounts.dmm.co.jp/"];
    let pending = Rc::new(RefCell::new((urls.len(), Some(reply))));
    for url in urls {
        let mut callback = DeletedCookies::new(pending.clone());
        if manager.delete_cookies(Some(&CefString::from(url)), None, Some(&mut callback)) == 0 {
            eprintln!("[web-avenger] não foi possível apagar os cookies de {url}");
        }
    }
}

type PendingReply = Rc<RefCell<(usize, Option<Box<dyn FnOnce(Value)>>)>>;

wrap_delete_cookies_callback! {
    struct DeletedCookies {
        pending: PendingReply,
    }

    impl DeleteCookiesCallback {
        fn on_complete(&self, _num_deleted: ::std::os::raw::c_int) {
            let reply = {
                let mut pending = self.pending.borrow_mut();
                pending.0 -= 1;
                if pending.0 > 0 { None } else { pending.1.take() }
            };
            if let Some(reply) = reply {
                reply(json!({ "ok": true }));
            }
        }
    }
}

/// Chamado em toda navegação (`RequestHandler::on_before_browse`). Encadeia os logins
/// (DMM → FANZA → Biblioteca) e retorna `true` quando a navegação deve ser cancelada.
pub fn handle_navigation(frame: &mut Frame, url: &str) -> bool {
    if url.starts_with(GENERAL_DONE) {
        // O FANZA tem sessão própria: segue para o login dele (se já houver sessão,
        // o DMM volta direto).
        frame.load_url(Some(&CefString::from(login_url("accounts.dmm.co.jp", ADULT_DONE).as_str())));
        return true;
    }
    if !url.starts_with(ADULT_DONE) {
        return false;
    }
    let frame = frame.clone();
    // Lê "Meus jogos" para confirmar o login e mostrar "Conectado como …".
    sync::fetch_my_games(Box::new(move |result| {
        let hash = match result {
            Ok(page) if page.logged_in => {
                format!("dmm=ok&user={}", http::url_encode(page.nickname.as_deref().unwrap_or("")))
            }
            Ok(_) => format!("dmm=erro&msg={}", http::url_encode("O login no DMM não foi concluído.")),
            Err(error) => format!("dmm=erro&msg={}", http::url_encode(&error)),
        };
        // Com a sessão já ativa, os logins redirecionam sem a Biblioteca sair da tela: a
        // query faz disso um carregamento novo (só o `#` seria uma navegação na mesma página).
        let url = format!("{}?retorno#{hash}", crate::library_url());
        frame.load_url(Some(&CefString::from(url.as_str())));
    }));
    true
}
