mod app;
mod catalog;
mod client;
mod library;

use cef::*;
use std::sync::OnceLock;

/// URL `data:` da Biblioteca, gerada uma vez no processo principal.
static LIBRARY_URL: OnceLock<String> = OnceLock::new();

pub fn library_url() -> &'static str {
    LIBRARY_URL.get().map(String::as_str).unwrap_or("about:blank")
}

fn main() {
    // Inicializa a versão da API do CEF.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let args = args::Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        eprintln!("[web-avenger] falha ao ler os argumentos da linha de comando");
        std::process::exit(1);
    };

    // O mesmo executável roda os subprocessos do Chromium (renderer, GPU, ...).
    let is_browser_process = cmd_line.has_switch(Some(&CefString::from("type"))) == 0;
    let mut app = app::AvengerApp::new();
    let code = execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if !is_browser_process {
        std::process::exit(code);
    }

    LIBRARY_URL.get_or_init(|| library::data_url(&catalog::load()));

    let data_dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("web-avenger");
    let profile_dir = data_dir.join("profile");
    let settings = Settings {
        no_sandbox: 1,
        root_cache_path: CefString::from(data_dir.to_string_lossy().as_ref()),
        cache_path: CefString::from(profile_dir.to_string_lossy().as_ref()),
        persist_session_cookies: 1,
        log_severity: LogSeverity::WARNING,
        background_color: 0xFF0B0E14,
        ..Default::default()
    };

    if initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    ) != 1
    {
        eprintln!("[web-avenger] falha ao inicializar o CEF");
        std::process::exit(1);
    }

    run_message_loop();
    shutdown();
}
