use cef::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const VK_ESCAPE: i32 = 0x1B;
const VK_F1: i32 = 0x70;
const VK_F5: i32 = 0x74;
const VK_F11: i32 = 0x7A;

/// Evento nativo de teclado que o CEF repassa em `on_pre_key_event`.
#[cfg(target_os = "linux")]
type OsKeyEvent = sys::XEvent;
#[cfg(windows)]
type OsKeyEvent = sys::MSG;

/// Browsers abertos (Biblioteca + janelas de jogos, ver `shell`).
static OPEN_BROWSERS: AtomicUsize = AtomicUsize::new(0);

fn window_for(browser: Option<&mut Browser>) -> Option<Window> {
    browser_view_get_for_browser(browser)?.window()
}

/// Quando o compositor tira a janela do fullscreen, `is_fullscreen()` continua
/// retornando 1; por isso também exige que a janela cubra o monitor.
pub fn is_window_fullscreen(window: &Window) -> bool {
    let covers_display = window.display().is_some_and(|display| {
        let (window, display) = (window.bounds(), display.bounds());
        window.width >= display.width && window.height >= display.height
    });
    window.is_fullscreen() != 0 && covers_display
}

wrap_client! {
    pub struct AvengerClient;

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(AvengerDisplayHandler::new())
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(AvengerKeyboardHandler::new())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(AvengerLifeSpanHandler::new())
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(AvengerRequestHandler::new())
        }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            match (frame, message) {
                (Some(frame), Some(message)) => {
                    (crate::page_info::handle_request(frame, message)
                        || crate::itch::handle_request(frame, message)
                        || crate::dmm::handle_request(frame, message)
                        || crate::explore::handle_request(frame, message)) as i32
                }
                _ => 0,
            }
        }
    }
}

wrap_display_handler! {
    struct AvengerDisplayHandler;

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let Some(window) = window_for(browser) else {
                return;
            };
            let title = title.map(CefString::to_string).unwrap_or_default();
            let title = if title.is_empty() || title.starts_with("data:") || title == "Web Avenger" {
                "Web Avenger".to_string()
            } else {
                format!("{title} — Web Avenger")
            };
            window.set_title(Some(&CefString::from(title.as_str())));
        }

        fn on_address_change(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            if frame.is_none_or(|frame| frame.is_main() == 0) {
                return;
            }
            let url = url.map(CefString::to_string).unwrap_or_default();
            if let Some(browser) = browser {
                crate::playtime::on_navigation(browser.identifier(), &url);
            }
        }

        fn on_fullscreen_mode_change(&self, browser: Option<&mut Browser>, fullscreen: i32) {
            // Fullscreen pedido pela própria página (Fullscreen API). Se a janela do jogo
            // abriu em tela cheia pela opção "Abrir jogos em tela cheia", continua nela.
            if fullscreen == 0 && browser.as_deref().is_some_and(crate::shell::is_auto_fullscreen) {
                return;
            }
            if let Some(window) = window_for(browser) {
                window.set_fullscreen(fullscreen);
            }
        }
    }
}

wrap_keyboard_handler! {
    struct AvengerKeyboardHandler;

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut OsKeyEvent>,
            _is_keyboard_shortcut: Option<&mut i32>,
        ) -> i32 {
            let (Some(browser), Some(event)) = (browser, event) else {
                return 0;
            };
            if event.type_ != KeyEventType::RAWKEYDOWN && event.type_ != KeyEventType::KEYDOWN {
                return 0;
            }

            // Fullscreen da página (Fullscreen API), que no estilo Alloy o app precisa encerrar.
            let host = browser.host();
            let in_page_fullscreen = host.as_ref().is_some_and(|host| host.is_fullscreen() != 0);

            match event.windows_key_code {
                VK_ESCAPE if in_page_fullscreen => {
                    if let Some(host) = host {
                        host.exit_fullscreen(1);
                    }
                }
                // Na janela de um jogo, o F1 vai para o jogo.
                VK_F1 if !crate::shell::is_game_browser(browser) => {
                    if let Some(frame) = browser.main_frame() {
                        frame.load_url(Some(&CefString::from(crate::library_url())));
                    }
                }
                VK_F5 => browser.reload_ignore_cache(),
                VK_F11 if in_page_fullscreen => {
                    // Sai da página e da janela juntas (via on_fullscreen_mode_change).
                    if let Some(host) = host {
                        host.exit_fullscreen(1);
                    }
                }
                VK_F11 => {
                    crate::shell::clear_auto_fullscreen(browser);
                    if let Some(window) = window_for(Some(browser)) {
                        window.set_fullscreen((!is_window_fullscreen(&window)) as i32);
                    }
                }
                _ => return 0,
            }
            1
        }
    }
}

wrap_life_span_handler! {
    struct AvengerLifeSpanHandler;

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            // Popups abrem na mesma janela em vez de criar janelas novas.
            if let (Some(frame), Some(url)) = (frame, target_url) {
                frame.load_url(Some(url));
            }
            1
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            OPEN_BROWSERS.fetch_add(1, Ordering::Relaxed);
            // Janela de jogo: bloqueio de anúncios, a menos que o jogo o desligue.
            if let Some(browser) = browser
                && let Some(id) = crate::shell::game_of(browser)
            {
                // Jogos fora do catálogo (abertos pelo Explorar) também bloqueiam.
                let enabled = !crate::catalog::load().iter().any(|game| game.id == id && game.no_adblock);
                crate::adblock::set_enabled(browser.identifier(), enabled);
            }
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                crate::playtime::on_browser_closed(browser.identifier());
                crate::adblock::set_enabled(browser.identifier(), false);
            }
            // A Biblioteca e cada jogo manual têm browsers separados (ver `shell`); o app
            // só sai quando o último fecha.
            if OPEN_BROWSERS.fetch_sub(1, Ordering::Relaxed) <= 1 {
                quit_message_loop();
            }
        }
    }
}

wrap_request_handler! {
    struct AvengerRequestHandler;

    impl RequestHandler {
        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _is_navigation: i32,
            _is_download: i32,
            _request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut i32>,
        ) -> Option<ResourceRequestHandler> {
            Some(AvengerResourceRequestHandler::new())
        }

        fn on_before_browse(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _user_gesture: i32,
            _is_redirect: i32,
        ) -> i32 {
            let (Some(browser), Some(frame), Some(request)) = (browser, frame, request) else {
                return 0;
            };
            let url = CefStringUtf16::from(&request.url()).to_string();
            // Retorno do login OAuth do itch.io / dos logins do DMM: concluídos aqui, a
            // navegação é cancelada.
            if crate::itch::handle_navigation(frame, &url) || crate::dmm::handle_navigation(frame, &url) {
                return 1;
            }
            if frame.is_main() == 0 {
                return 0;
            }

            // Jogo aberto pela Biblioteca: vai para a janela própria dele.
            if crate::shell::is_library_browser(browser)
                && let Some(game) = crate::playtime::catalog_game_for(&url)
            {
                post_task(ThreadId::UI, Some(&mut crate::shell::OpenGameTask::new(game, url)));
                return 1;
            }
            0
        }
    }
}

wrap_resource_request_handler! {
    struct AvengerResourceRequestHandler;

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            // Roda na thread de IO: o `adblock` decide sem consultar o `shell`.
            if let (Some(browser), Some(request)) = (browser, request) {
                let url = CefStringUtf16::from(&request.url()).to_string();
                let source = frame.map(|frame| CefStringUtf16::from(&frame.url()).to_string()).unwrap_or_default();
                if crate::adblock::should_block(browser.identifier(), &url, &source, request.resource_type()) {
                    return ReturnValue::CANCEL;
                }
            }
            // O padrão do cef-rs (0) seria `RV_CANCEL`.
            ReturnValue::CONTINUE
        }
    }
}
