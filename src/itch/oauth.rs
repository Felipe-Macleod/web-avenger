//! Login no itch.io por OAuth (authorization code + PKCE, sem client secret).
//!
//! A página de autorização abre na própria janela do app. Ao autorizar, o itch.io
//! redireciona para `REDIRECT_URI`; nada escuta nesse endereço: o `RequestHandler`
//! do cliente intercepta a navegação, pega o `code` e troca pelo token aqui.

use crate::http;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// Registrada no app OAuth em https://itch.io/user/settings/oauth-apps.
pub const REDIRECT_URI: &str = "http://127.0.0.1/web-avenger/oauth/callback";
/// Só escopos concedíveis por OAuth (https://itch.io/docs/api/oauth). Os jogos de uma
/// coleção (`/collections/{id}/collection-games`) pedem `collection:view`, que o OAuth
/// não concede: eles são lidos da página da coleção (ver `sync.rs`).
pub const SCOPES: &str = "profile:me profile:owned profile:collections game:view:uploads";
const AUTHORIZE_URL: &str = "https://itch.io/user/oauth";
const TOKEN_URL: &str = "https://api.itch.io/oauth/token";
const REVOKE_URL: &str = "https://api.itch.io/oauth/revoke";
/// O código de autorização do itch.io expira em 10 minutos.
const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

/// `ITCH_CLIENT_ID` do ambiente (prioridade) ou o embutido no build (`build.rs`).
pub fn client_id() -> Option<String> {
    std::env::var("ITCH_CLIENT_ID")
        .ok()
        .filter(|id| !id.trim().is_empty())
        .or_else(|| option_env!("ITCH_CLIENT_ID").map(str::to_string))
        .map(|id| id.trim().to_string())
}

struct Pending {
    verifier: String,
    state: String,
    created: Instant,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn random_token(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::getrandom(&mut buffer).map_err(|e| format!("sem fonte de aleatoriedade: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

/// `code_challenge` S256 do PKCE (RFC 7636).
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn authorize_url(client_id: &str, state: &str, challenge: &str) -> String {
    let query = http::encode_form(&[
        ("client_id", client_id),
        ("scope", SCOPES),
        ("redirect_uri", REDIRECT_URI),
        ("response_type", "code"),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ]);
    format!("{AUTHORIZE_URL}?{query}")
}

/// Inicia um login: guarda verifier/state e devolve a URL de autorização.
pub fn begin(client_id: &str) -> Result<String, String> {
    let verifier = random_token(32)?;
    let state = random_token(16)?;
    let url = authorize_url(client_id, &state, &pkce_challenge(&verifier));
    *PENDING.lock().map_err(|e| e.to_string())? = Some(Pending { verifier, state, created: Instant::now() });
    Ok(url)
}

/// Resultado de uma navegação para a `REDIRECT_URI`.
#[derive(Debug, PartialEq)]
pub enum Redirect {
    Code { code: String, state: String },
    Error(String),
}

/// `None` se a URL não é a de retorno do OAuth.
pub fn parse_redirect(url: &str) -> Option<Redirect> {
    let rest = url.strip_prefix(REDIRECT_URI)?;
    if !(rest.is_empty() || rest.starts_with('?') || rest.starts_with('#')) {
        return None;
    }
    let query = rest.trim_start_matches(['?', '#']).split('#').next().unwrap_or("");
    let params = http::parse_query(query);
    let get = |name: &str| params.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
    Some(match (get("code"), get("state"), get("error")) {
        (_, _, Some(error)) => Redirect::Error(match error.as_str() {
            "access_denied" => "Autorização negada no itch.io.".into(),
            _ => get("error_description").unwrap_or(error),
        }),
        (Some(code), Some(state), None) => Redirect::Code { code, state },
        _ => Redirect::Error("Resposta do itch.io sem código de autorização.".into()),
    })
}

/// Confere o `state` e devolve o `code_verifier` do login em andamento (uso único).
pub fn take_verifier(state: &str) -> Result<String, String> {
    let pending = PENDING.lock().map_err(|e| e.to_string())?.take();
    match pending {
        Some(p) if p.state == state && p.created.elapsed() < PENDING_TTL => Ok(p.verifier),
        Some(p) if p.state == state => Err("O login expirou; tente conectar de novo.".into()),
        _ => Err("Resposta de login inesperada (state não confere).".into()),
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    errors: Vec<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Troca o código pelo token: `Ok((token, escopos))`.
pub fn exchange(
    client_id: &str,
    code: &str,
    verifier: &str,
    done: Box<dyn FnOnce(Result<(String, Vec<String>), String>)>,
) {
    let form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", REDIRECT_URI),
        ("client_id", client_id),
    ];
    http::fetch(
        http::Request::post_form(TOKEN_URL, form),
        Box::new(move |result| {
            done(result.and_then(|response| {
                let parsed: TokenResponse = serde_json::from_slice(&response.body)
                    .map_err(|_| format!("resposta inesperada do itch.io (HTTP {})", response.status))?;
                match parsed.access_token {
                    Some(token) if response.ok() => {
                        let scopes = parsed.scope.unwrap_or_default().split_whitespace().map(str::to_string).collect();
                        Ok((token, scopes))
                    }
                    _ => Err(match parsed.errors.into_iter().next().or(parsed.error).as_deref() {
                        Some("invalid_grant") => {
                            "O código de autorização é inválido ou expirou; tente conectar de novo.".into()
                        }
                        Some("invalid_client") => {
                            "O itch.io não reconheceu o ITCH_CLIENT_ID; confira o app OAuth.".into()
                        }
                        Some(other) => format!("O itch.io recusou o login: {other}"),
                        None => format!("O itch.io recusou o login (HTTP {}).", response.status),
                    }),
                }
            }))
        }),
    );
}

/// Revoga o token no itch.io (melhor esforço: o token local é apagado de qualquer forma).
pub fn revoke(client_id: &str, token: &str, done: Box<dyn FnOnce()>) {
    http::fetch(
        http::Request::post_form(REVOKE_URL, vec![("token", token), ("client_id", client_id)]),
        Box::new(move |_| done()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_vetor_da_rfc_7636() {
        // RFC 7636, apêndice B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn url_de_autorizacao() {
        let url = authorize_url("abc123", "st", "ch");
        assert!(url.starts_with("https://itch.io/user/oauth?client_id=abc123&scope=profile%3Ame%20profile%3Aowned"));
        assert!(url.contains("&redirect_uri=http%3A%2F%2F127.0.0.1%2Fweb-avenger%2Foauth%2Fcallback"));
        assert!(url.contains("&response_type=code&state=st&code_challenge=ch&code_challenge_method=S256"));
    }

    #[test]
    fn interpreta_o_retorno() {
        assert_eq!(parse_redirect("https://itch.io/x"), None);
        assert_eq!(parse_redirect(&format!("{REDIRECT_URI}extra")), None);
        assert_eq!(
            parse_redirect(&format!("{REDIRECT_URI}?code=c%2B1&state=s")),
            Some(Redirect::Code { code: "c+1".into(), state: "s".into() })
        );
        assert_eq!(
            parse_redirect(&format!("{REDIRECT_URI}?error=access_denied&state=s")),
            Some(Redirect::Error("Autorização negada no itch.io.".into()))
        );
        assert!(matches!(parse_redirect(&format!("{REDIRECT_URI}?state=s")), Some(Redirect::Error(_))));
    }

    #[test]
    fn state_e_verifier() {
        let url = begin("id").unwrap();
        let state = http::parse_query(url.split_once('?').unwrap().1)
            .into_iter()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1;
        assert!(take_verifier("outro").is_err());
        // O pedido pendente é de uso único: depois de uma tentativa, some.
        begin("id").unwrap();
        let url = begin("id").unwrap();
        let state2 = http::parse_query(url.split_once('?').unwrap().1).into_iter().find(|(k, _)| k == "state").unwrap().1;
        assert_ne!(state, state2);
        let verifier = take_verifier(&state2).unwrap();
        assert_eq!(verifier.len(), 43);
        assert!(take_verifier(&state2).is_err());
    }
}
