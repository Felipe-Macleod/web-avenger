//! Código que roda no processo de renderização (um por página).

use crate::{audio, catalog, library, native_pads, page_info, settings};
use cef::*;
use std::{cell::RefCell, collections::HashMap};

thread_local! {
    /// Pedidos de `avengerFetchPageInfo` aguardando resposta do processo principal.
    static PENDING_PAGE_INFO: RefCell<(i32, HashMap<i32, (V8Context, V8Value)>)> =
        RefCell::new((0, HashMap::new()));
}

const INPUT_TRANSLATION: &str = include_str!("../assets/input-translation.js");
const PLAYER_FIT: &str = include_str!("../assets/player-fit.js");
const AUDIO_FALLBACK: &str = include_str!("../assets/audio-fallback.js");

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

            if !is_library {
                // Lida e removida do `window` pelo script de fallback de áudio.
                install_function(context, "__avengerDecodeAudio", &mut DecodeAudioHandler::new());
                eval(context, AUDIO_FALLBACK);
            }
            if !is_library && settings.maximize_player {
                eval(context, PLAYER_FIT);
            }
            if !is_library && !settings.input_translation {
                return;
            }
            // Lida e removida do `window` pelo tradutor de input.
            install_function(context, "__avengerNativePads", &mut NativePadsHandler::new());

            if is_library {
                // A Biblioteca sempre recebe o tradutor (para o testador de controle)
                // e a ponte para ler/salvar as configurações.
                install_function(context, "avengerSaveSettings", &mut SaveSettingsHandler::new());
                install_function(context, "avengerAddGame", &mut AddGameHandler::new());
                install_function(context, "avengerFetchPageInfo", &mut FetchPageInfoHandler::new());
                let json = serde_json::to_string(&settings).unwrap_or_default();
                eval(context, &format!("window.avengerSettings = {json};"));
            }
            eval(context, &input_translation_script(&settings, is_library));
        }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            let Some(message) = message else {
                return 0;
            };
            if CefStringUtf16::from(&message.name()).to_string() != page_info::RESPONSE_MESSAGE {
                return 0;
            }
            let Some(args) = message.argument_list() else {
                return 1;
            };
            let pending = PENDING_PAGE_INFO.with(|pending| pending.borrow_mut().1.remove(&args.int(0)));
            let Some((context, callback)) = pending else {
                return 1;
            };
            // A Biblioteca pode ter navegado para outro lugar enquanto a página baixava.
            if context.is_valid() != 0 && context.enter() != 0 {
                let json = v8_value_create_string(Some(&CefString::from(&args.string(1))));
                callback.execute_function(None, Some(&[json]));
                context.exit();
            }
            1
        }
    }
}

wrap_v8_handler! {
    struct DecodeAudioHandler;

    impl V8Handler {
        /// `__avengerDecodeAudio(mp4: ArrayBuffer) -> ArrayBuffer` (ver `audio::Decoded::to_bytes`)
        fn execute(
            &self,
            _name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<V8Value>>,
            exception: Option<&mut CefString>,
        ) -> i32 {
            let input = arguments
                .and_then(|args| args.first().cloned().flatten())
                .filter(|arg| arg.is_array_buffer() != 0)
                .map(|arg| {
                    let (data, len) = (arg.array_buffer_data(), arg.array_buffer_byte_length());
                    if data.is_null() || len == 0 {
                        return Vec::new();
                    }
                    // SAFETY: ponteiro e tamanho vêm do próprio ArrayBuffer, vivo durante a chamada.
                    unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len) }.to_vec()
                });

            let result = input
                .ok_or_else(|| "esperado um ArrayBuffer".to_string())
                .and_then(audio::decode)
                .map(|decoded| decoded.to_bytes());
            match result {
                Ok(mut bytes) => {
                    if let Some(retval) = retval {
                        *retval = v8_value_create_array_buffer_with_copy(bytes.as_mut_ptr(), bytes.len());
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
    struct FetchPageInfoHandler;

    impl V8Handler {
        /// `avengerFetchPageInfo(url: string, callback: (json: string) => void)`
        fn execute(
            &self,
            _name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            _retval: Option<&mut Option<V8Value>>,
            exception: Option<&mut CefString>,
        ) -> i32 {
            let url = string_argument(arguments);
            let callback = arguments
                .and_then(|args| args.get(1).cloned().flatten())
                .filter(|arg| arg.is_function() != 0);
            let context = v8_context_get_current_context();
            let frame = context.as_ref().and_then(|context| context.frame());

            let (Ok(url), Some(callback), Some(context), Some(frame)) = (url, callback, context, frame) else {
                if let Some(exception) = exception {
                    *exception = CefString::from("uso: avengerFetchPageInfo(url, callback)");
                }
                return 1;
            };
            let Some(mut message) = process_message_create(Some(&CefString::from(page_info::REQUEST_MESSAGE))) else {
                return 1;
            };
            let id = PENDING_PAGE_INFO.with(|pending| {
                let (next_id, map) = &mut *pending.borrow_mut();
                *next_id += 1;
                map.insert(*next_id, (context, callback));
                *next_id
            });
            if let Some(args) = message.argument_list() {
                args.set_int(0, id);
                args.set_string(1, Some(&CefString::from(url.as_str())));
            }
            frame.send_process_message(ProcessId::BROWSER, Some(&mut message));
            1
        }
    }
}

/// Primeiro argumento de uma função nativa, se for uma string.
fn string_argument(arguments: Option<&[Option<V8Value>]>) -> Result<String, String> {
    arguments
        .and_then(|args| args.first().cloned().flatten())
        .filter(|arg| arg.is_string() != 0)
        .map(|arg| CefStringUtf16::from(&arg.string_value()).to_string())
        .ok_or_else(|| "esperado um JSON (string)".to_string())
}

wrap_v8_handler! {
    struct AddGameHandler;

    impl V8Handler {
        /// `avengerAddGame(json: string) -> string` (o jogo salvo, com `id`)
        fn execute(
            &self,
            _name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<V8Value>>,
            exception: Option<&mut CefString>,
        ) -> i32 {
            let result = string_argument(arguments)
                .and_then(|json| serde_json::from_str::<catalog::Game>(&json).map_err(|e| e.to_string()))
                .and_then(|game| catalog::add_user_game(game).map_err(|e| e.to_string()))
                .and_then(|game| {
                    // Regera o arquivo da Biblioteca para o jogo continuar lá no F1/F5.
                    library::write(&catalog::load()).map_err(|e| e.to_string())?;
                    serde_json::to_string(&game).map_err(|e| e.to_string())
                });

            match result {
                Ok(json) => {
                    if let Some(retval) = retval {
                        *retval = v8_value_create_string(Some(&CefString::from(json.as_str())));
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
            let result = string_argument(arguments)
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
