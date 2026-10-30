use cef::*;

const VK_ESCAPE: i32 = 0x1B;
const VK_F1: i32 = 0x70;
const VK_F5: i32 = 0x74;
const VK_F11: i32 = 0x7A;

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

        fn on_fullscreen_mode_change(&self, browser: Option<&mut Browser>, fullscreen: i32) {
            // Fullscreen pedido pela própria página (Fullscreen API).
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
            _os_event: Option<&mut sys::XEvent>,
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
                VK_F1 => {
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

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            quit_message_loop();
        }
    }
}
