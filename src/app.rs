use crate::client::{AvengerClient, is_window_fullscreen};
use crate::renderer::AvengerRenderProcessHandler;
use cef::*;
use std::cell::RefCell;

/// Flags do Chromium necessárias para jogos WebGPU/WASM no Linux.
const SWITCHES: &[(&str, Option<&str>)] = &[
    ("enable-unsafe-webgpu", None),
    ("ignore-gpu-blocklist", None),
    ("enable-gpu-rasterization", None),
    // Sem a feature `Vulkan`, o WebGPU no Linux cai no SwiftShader (CPU, ~2 FPS).
    ("enable-features", Some("Vulkan")),
    ("autoplay-policy", Some("no-user-gesture-required")),
    // Views do CEF em Wayland nativo não acompanha os resizes do compositor
    // (fullscreen e tiling ficam com o conteúdo preso no tamanho inicial).
    ("ozone-platform", Some("x11")),
    // No estilo Chrome, inicializar um perfil novo (jogos manuais, ver `profiles`)
    // abriria uma janela "Nova guia" do Chromium. (`no-startup-window` também evitaria,
    // mas mantém o processo vivo depois de fechar a janela.)
    ("no-first-run", None),
];

wrap_app! {
    pub struct AvengerApp;

    impl App {
        fn on_before_command_line_processing(
            &self,
            process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            // As flags do processo principal são repassadas aos subprocessos.
            let is_browser = process_type.map(|t| t.to_string().is_empty()).unwrap_or(true);
            let Some(command_line) = command_line.filter(|_| is_browser) else {
                return;
            };
            for (name, value) in SWITCHES {
                let name = CefString::from(*name);
                match value {
                    Some(value) => command_line
                        .append_switch_with_value(Some(&name), Some(&CefString::from(*value))),
                    None => command_line.append_switch(Some(&name)),
                }
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(AvengerBrowserProcessHandler::new())
        }

        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(AvengerRenderProcessHandler::new())
        }
    }
}

wrap_browser_process_handler! {
    struct AvengerBrowserProcessHandler;

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            let mut client = AvengerClient::new();
            let url = CefString::from(crate::library_url());
            let settings = BrowserSettings::default();
            let mut delegate = AvengerBrowserViewDelegate::new();
            let browser_view = browser_view_create(
                Some(&mut client),
                Some(&url),
                Some(&settings),
                None,
                None,
                Some(&mut delegate),
            );

            let mut delegate = AvengerWindowDelegate::new(RefCell::new(browser_view));
            window_create_top_level(Some(&mut delegate));
        }
    }
}

wrap_window_delegate! {
    struct AvengerWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1280, height: 800 }
        }

        fn minimum_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 640, height: 400 }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let browser_view = self.browser_view.borrow();
            let (Some(window), Some(browser_view)) = (window, browser_view.as_ref()) else {
                return;
            };
            window.set_title(Some(&CefString::from("Web Avenger")));
            let mut view = View::from(browser_view);
            window.add_child_view(Some(&mut view));
            window.show();
            browser_view.request_focus();
            crate::shell::init(window.clone(), browser_view.clone());
        }

        fn on_window_fullscreen_transition(&self, window: Option<&mut Window>, is_completed: i32) {
            if is_completed != 0 {
                self.sync_page_fullscreen(window);
            }
        }

        fn on_window_bounds_changed(&self, window: Option<&mut Window>, _new_bounds: Option<&Rect>) {
            // O compositor pode tirar a janela do fullscreen sem passar pela transição acima.
            self.sync_page_fullscreen(window);
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            crate::playtime::end_all();
            *self.browser_view.borrow_mut() = None;
            crate::shell::clear();
            // Perfis de jogos ainda referenciados seguram o processo no estilo Chrome:
            // sem soltá-los aqui, o `run_message_loop` não termina.
            crate::profiles::release_all();
        }

        // No cef-rs, os métodos não implementados retornam 0 (no CEF em C++ o padrão é
        // `true`). Uma janela sem `can_resize` vai para o X com tamanho mínimo = máximo,
        // e o Hyprland a trata como fixa: flutuante e na posição pedida pelo Chromium
        // (canto do monitor principal), em vez de lado a lado no monitor focado.
        fn can_resize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_maximize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_minimize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            // Fecha o jogo manual aberto (se houver) e a Biblioteca.
            crate::shell::close_all()
        }
    }
}

impl AvengerWindowDelegate {
    /// Se a janela saiu do fullscreen (F11, compositor...), a página também deve sair.
    fn sync_page_fullscreen(&self, window: Option<&mut Window>) {
        if window.is_none_or(|window| is_window_fullscreen(window)) {
            return;
        }
        let browser = crate::shell::active_view().and_then(|view| view.browser());
        if let Some(host) = browser.and_then(|browser| browser.host())
            && host.is_fullscreen() != 0
        {
            host.exit_fullscreen(1);
        }
    }
}

wrap_browser_view_delegate! {
    pub struct AvengerBrowserViewDelegate {}

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            // Estilo Alloy: só a página, sem a interface do Chrome.
            RuntimeStyle::ALLOY
        }
    }
}
