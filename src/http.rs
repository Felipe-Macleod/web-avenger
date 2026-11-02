//! Requisições HTTP do processo principal (`CefURLRequest`, fora das regras de CORS).
//!
//! Devem ser iniciadas na thread de UI do processo principal; o callback roda na
//! mesma thread quando a resposta termina.

use cef::*;
use std::sync::{Arc, Mutex};

/// Respostas maiores que isso são truncadas (o suficiente para páginas e JSON da API).
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// `UR_FLAG_ALLOW_STORED_CREDENTIALS`: envia os cookies do perfil (ex.: login no itch.io).
const FLAG_COOKIES: i32 = 8;

pub struct Response {
    pub status: i32,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub type Callback = Box<dyn FnOnce(Result<Response, String>)>;

#[derive(Default)]
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: Vec<(&'a str, String)>,
    /// Corpo `application/x-www-form-urlencoded`.
    pub form: Option<Vec<(&'a str, &'a str)>>,
    /// Envia os cookies do perfil do navegador.
    pub cookies: bool,
}

impl<'a> Request<'a> {
    pub fn get(url: &'a str) -> Self {
        Self { method: "GET", url, ..Default::default() }
    }

    pub fn post_form(url: &'a str, form: Vec<(&'a str, &'a str)>) -> Self {
        Self { method: "POST", url, form: Some(form), ..Default::default() }
    }

    pub fn bearer(mut self, token: &str) -> Self {
        self.headers.push(("Authorization", format!("Bearer {token}")));
        self
    }
}

/// Inicia a requisição. Se não for possível iniciar, o callback é chamado na hora com erro.
pub fn fetch(request: Request, callback: Callback) {
    let callback = Arc::new(Mutex::new(Some(callback)));
    let mut client = HttpClient::new(Arc::default(), callback.clone());
    if !start(&request, &mut client) {
        finish(&callback, Err(crate::i18n::tf("error.request", &[("url", &request.url)])));
    }
}

fn start(request: &Request, client: &mut UrlrequestClient) -> bool {
    let Some(mut cef_request) = request_create() else {
        return false;
    };
    cef_request.set_url(Some(&CefString::from(request.url)));
    cef_request.set_method(Some(&CefString::from(request.method)));
    if request.cookies {
        cef_request.set_flags(FLAG_COOKIES);
    }
    for (name, value) in &request.headers {
        cef_request.set_header_by_name(
            Some(&CefString::from(*name)),
            Some(&CefString::from(value.as_str())),
            1,
        );
    }
    if let Some(form) = &request.form {
        let body = encode_form(form);
        let (Some(mut post), Some(mut element)) = (post_data_create(), post_data_element_create()) else {
            return false;
        };
        element.set_to_bytes(body.len(), body.as_ptr());
        post.add_element(Some(&mut element));
        cef_request.set_post_data(Some(&mut post));
        cef_request.set_header_by_name(
            Some(&CefString::from("Content-Type")),
            Some(&CefString::from("application/x-www-form-urlencoded")),
            1,
        );
    }
    urlrequest_create(Some(&mut cef_request), Some(client), None).is_some()
}

fn finish(callback: &Mutex<Option<Callback>>, result: Result<Response, String>) {
    let callback = callback.lock().ok().and_then(|mut callback| callback.take());
    if let Some(callback) = callback {
        callback(result);
    }
}

wrap_urlrequest_client! {
    struct HttpClient {
        body: Arc<Mutex<Vec<u8>>>,
        callback: Arc<Mutex<Option<Callback>>>,
    }

    impl UrlrequestClient {
        fn on_download_data(&self, _request: Option<&mut Urlrequest>, data: *const u8, data_length: usize) {
            if data.is_null() {
                return;
            }
            // SAFETY: o CEF garante `data_length` bytes válidos durante a chamada.
            let chunk = unsafe { std::slice::from_raw_parts(data, data_length) };
            if let Ok(mut body) = self.body.lock() {
                let room = MAX_BYTES.saturating_sub(body.len());
                body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            }
        }

        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let result = match request {
                Some(request) if request.request_status() == UrlrequestStatus::SUCCESS => {
                    let status = request.response().map_or(0, |response| response.status());
                    let body = self.body.lock().map(|mut body| std::mem::take(&mut *body)).unwrap_or_default();
                    Ok(Response { status, body })
                }
                _ => Err(crate::i18n::t("error.network")),
            };
            finish(&self.callback, result);
        }
    }
}

/// Codifica para URL (query string e formulários), no estilo `application/x-www-form-urlencoded`.
pub fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn encode_form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", url_encode(key), url_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Decodifica um componente de query string (`%XX` e `+`).
pub fn url_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).and_then(|hex| std::str::from_utf8(hex).ok());
                match hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Pares `chave=valor` de uma query string (sem o `?`).
pub fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (url_decode(key), url_decode(value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codifica_e_decodifica_url() {
        assert_eq!(url_encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(url_decode("a%20b+c%26%C3%A9"), "a b c&é");
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(encode_form(&[("grant_type", "authorization_code"), ("x", "1 2")]), "grant_type=authorization_code&x=1%202");
        assert_eq!(
            parse_query("code=abc%2B1&state=xyz&flag"),
            [("code".into(), "abc+1".into()), ("state".into(), "xyz".into()), ("flag".into(), String::new())]
        );
    }
}
