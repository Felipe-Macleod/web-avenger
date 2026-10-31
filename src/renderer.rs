//! Código que roda no processo de renderização (um por página).

use crate::{library, native_pads, settings};
use cef::*;

const INPUT_TRANSLATION: &str = include_str!("../assets/input-translation.js");

/// Script do tradutor de input com a configuração embutida.
fn input_translation_script(settings: &settings::Settings, expose: bool) -> String {
    let config = serde_json::json!({
        "enabled": settings.input_translation,
        "layout": settings.button_layout,
        "mappings": settings.mappings,
        "expose": expose,
    });
    format!("{INPUT_TRANSLATION}\n({config});")
}

/// Registra uma função nativa no `window` da página.
fn install_function(context: &mut V8Context, name: &str, handler: &mut V8Handler) {
    let Some(global) = context.global() else {
        return;
    };
    let name = CefString::from(name);
    let mut function = v8_value_create_function(Some(&name), Some(handler));
    global.set_value_bykey(Some(&name), function.as_mut(), Default::default());
}

fn eval(context: &mut V8Context, code: &str) {
    let code = CefString::from(code);
    let url = CefString::from("web-avenger://injected");
    // O CEF recusa `retval` nulo, mesmo quando o resultado não interessa.
    let (mut retval, mut exception) = (None, None);
    if context.eval(Some(&code), Some(&url), 1, Some(&mut retval), Some(&mut exception)) == 0 {
        let message = exception.map(|e| CefStringUtf16::from(&e.message()).to_string()).unwrap_or_default();
        eprintln!("[web-avenger] erro no script injetado: {message}");
    }
}

wrap_render_process_handler! {
    pub struct AvengerRenderProcessHandler;

    impl RenderProcessHandler {
        fn on_context_created(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            context: Option<&mut V8Context>,
        ) {
            let (Some(frame), Some(context)) = (frame, context) else {
                return;
            };
            let url = CefStringUtf16::from(&frame.url()).to_string();
            let is_library = frame.is_main() != 0 && library::is_library_url(&url);
            let settings = settings::load();

            if !is_library && !settings.input_translation {
                return;
            }
            // Lida e removida do `window` pelo tradutor de input.
            install_function(context, "__avengerNativePads", &mut NativePadsHandler::new());

            if is_library {
                // A Biblioteca sempre recebe o tradutor (para o testador de controle)
                // e a ponte para ler/salvar as configurações.
                install_function(context, "avengerSaveSettings", &mut SaveSettingsHandler::new());
                let json = serde_json::to_string(&settings).unwrap_or_default();
                eval(context, &format!("window.avengerSettings = {json};"));
            }
            eval(context, &input_translation_script(&settings, is_library));
        }
    }
}

wrap_v8_handler! {
    struct SaveSettingsHandler;

    impl V8Handler {
        /// `avengerSaveSettings(json: string) -> boolean`
        fn execute(
            &self,
            _name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<V8Value>>,
            exception: Option<&mut CefString>,
        ) -> i32 {
            let json = arguments
                .and_then(|args| args.first().cloned().flatten())
                .filter(|arg| arg.is_string() != 0)
                .map(|arg| CefStringUtf16::from(&arg.string_value()).to_string());

            let result = json
                .ok_or_else(|| "esperado um JSON (string)".to_string())
                .and_then(|json| {
                    serde_json::from_str::<settings::Settings>(&json).map_err(|e| e.to_string())
                })
                .and_then(|value| settings::save(&value).map_err(|e| e.to_string()));

            match result {
                Ok(()) => {
                    if let Some(retval) = retval {
                        *retval = v8_value_create_bool(1);
                    }
                }
                Err(err) => {
                    if let Some(exception) = exception {
                        *exception = CefString::from(err.as_str());
                    }
                }
            }
            1
        }
    }
}

wrap_v8_handler! {
    struct NativePadsHandler;

    impl V8Handler {
        /// `__avengerNativePads() -> string` (JSON de `NativePad[]`)
        fn execute(
            &self,
            _name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            _arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<V8Value>>,
            _exception: Option<&mut CefString>,
        ) -> i32 {
            let json = serde_json::to_string(&native_pads::snapshot()).unwrap_or_default();
            if let Some(retval) = retval {
                *retval = v8_value_create_string(Some(&CefString::from(json.as_str())));
            }
            1
        }
    }
}
