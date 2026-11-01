//! Janelas do app: a da Biblioteca (perfil principal) e uma por jogo aberto.
//!
//! Jogos manuais rodam no perfil próprio (ver `profiles`); os importados e os do
//! catálogo embutido, no perfil principal (precisam da sessão do itch.io/DMM). Um jogo
//! tem no máximo uma janela: o CEF não aceita dois browsers na mesma pasta de perfil.
//! Fechar a Biblioteca não fecha os jogos; o app sai quando o último browser fecha (ver
//! `client.rs`). Tudo roda na thread de UI do processo principal.

use crate::{
    app::{AvengerBrowserViewDelegate, AvengerWindowDelegate},
    catalog::Game,
    client::AvengerClient,
    profiles,
};
use cef::*;
use std::{cell::RefCell, collections::HashMap};

struct GameWindow {
    view: BrowserView,
    /// `None` até o CEF criar a janela (`on_window_created`).
    window: Option<Window>,
    /// Se a tela cheia foi ligada pela opção "Abrir jogos em tela cheia".
    auto_fullscreen: bool,
}

#[derive(Default)]
struct Shell {
    library: Option<(Window, BrowserView)>,
    /// Janela de cada jogo aberto (id do jogo → janela).
    games: HashMap<String, GameWindow>,
}

thread_local! {
    static SHELL: RefCell<Shell> = RefCell::new(Shell::default());
}

fn same_browser(view: &BrowserView, browser: &Browser) -> bool {
    view.browser().is_some_and(|own| own.is_same(Some(&mut browser.clone())) != 0)
}

/// Abre a janela da Biblioteca (ou traz para frente a que já está aberta).
pub fn open_library() {
    if let Some(window) = SHELL.with(|shell| shell.borrow().library.as_ref().map(|(window, _)| window.clone())) {
        return window.activate();
    }
    let mut client = AvengerClient::new();
    let mut delegate = AvengerBrowserViewDelegate::new();
    let view = browser_view_create(
        Some(&mut client),
        Some(&CefString::from(crate::library_url())),
        Some(&BrowserSettings::default()),
        None,
        None,
        Some(&mut delegate),
    );
    let mut delegate = AvengerWindowDelegate::new(RefCell::new(view), None, "Web Avenger".into());
    window_create_top_level(Some(&mut delegate));
}

/// Abre o que a linha de comando pede: o jogo de `--game=<id>` (atalho da área de
/// trabalho) ou a Biblioteca.
pub fn open_from_command_line(command_line: &CommandLine) {
    let id = CefStringUtf16::from(&command_line.switch_value(Some(&CefString::from(crate::shortcut::GAME_SWITCH))))
        .to_string();
    match crate::catalog::load().into_iter().find(|game| !id.is_empty() && game.id == id) {
        Some(game) => {
            let url = game.url.clone();
            open_game(game, url);
        }
        None => open_library(),
    }
}

pub fn library_created(window: Window, view: BrowserView) {
    SHELL.with(|shell| shell.borrow_mut().library = Some((window, view)));
}

pub fn library_closed() {
    SHELL.with(|shell| shell.borrow_mut().library = None);
    release_profiles_if_last();
}

/// Perfis de jogos ainda referenciados seguram o processo no estilo Chrome: sem soltá-los
/// quando a última janela fecha, o `run_message_loop` não termina. Enquanto houver
/// janelas, ficam guardados para reabrir o jogo no mesmo perfil.
fn release_profiles_if_last() {
    if SHELL.with(|shell| shell.borrow().library.is_none() && shell.borrow().games.is_empty()) {
        profiles::release_all();
    }
}

/// Abre o jogo na janela dele (ou traz para frente a que já está aberta).
pub fn open_game(game: Game, url: String) {
    let existing = SHELL.with(|shell| shell.borrow().games.get(&game.id).and_then(|game| game.window.clone()));
    if let Some(window) = existing {
        return window.activate();
    }
    if SHELL.with(|shell| shell.borrow().games.contains_key(&game.id)) {
        return; // janela ainda sendo criada
    }
    if game.source.is_none() {
        let (id, name) = (game.id.clone(), game.name.clone());
        profiles::with_context(&game, Box::new(move |context| create_game_window(&id, &name, Some(context), &url)));
    } else {
        create_game_window(&game.id, &game.name, None, &url);
    }
}

fn create_game_window(id: &str, name: &str, mut context: Option<RequestContext>, url: &str) {
    let mut client = AvengerClient::new();
    let mut delegate = AvengerBrowserViewDelegate::new();
    let Some(view) = browser_view_create(
        Some(&mut client),
        Some(&CefString::from(url)),
        Some(&BrowserSettings::default()),
        None,
        context.as_mut(),
        Some(&mut delegate),
    ) else {
        eprintln!("[web-avenger] não foi possível abrir {url}");
        return;
    };
    // Registrado antes de criar a janela, e sem manter o `RefCell` emprestado: a criação
    // dispara callbacks (ex.: `on_before_browse`) que o consultam.
    let auto_fullscreen = crate::settings::load().fullscreen_games;
    SHELL.with(|shell| {
        shell
            .borrow_mut()
            .games
            .insert(id.to_string(), GameWindow { view: view.clone(), window: None, auto_fullscreen })
    });
    let mut delegate = AvengerWindowDelegate::new(RefCell::new(Some(view)), Some(id.to_string()), name.to_string());
    window_create_top_level(Some(&mut delegate));
}

/// A janela do jogo `id` foi criada; liga a tela cheia se a opção pedir.
pub fn game_window_created(id: &str, window: &Window) {
    let auto_fullscreen = SHELL.with(|shell| {
        let mut shell = shell.borrow_mut();
        let game = shell.games.get_mut(id)?;
        game.window = Some(window.clone());
        Some(game.auto_fullscreen)
    });
    if auto_fullscreen == Some(true) {
        window.set_fullscreen(1);
    }
}

/// A janela do jogo `id` fechou: recarrega a Biblioteca (para atualizar "Recentes").
pub fn game_window_closed(id: &str) {
    let library = SHELL.with(|shell| {
        let mut shell = shell.borrow_mut();
        shell.games.remove(id);
        shell.library.as_ref().map(|(_, view)| view.clone())
    });
    release_profiles_if_last();
    if let Some(frame) = library.and_then(|view| view.browser()).and_then(|browser| browser.main_frame()) {
        frame.load_url(Some(&CefString::from(crate::library_url())));
    }
}

/// Se o browser é o da Biblioteca.
pub fn is_library_browser(browser: &Browser) -> bool {
    SHELL.with(|shell| shell.borrow().library.as_ref().map(|(_, view)| view.clone())).is_some_and(|view| same_browser(&view, browser))
}

/// Id do jogo cuja janela tem o browser. Os views são copiados antes da comparação
/// para não manter o `RefCell` emprestado durante chamadas ao CEF.
fn game_of(browser: &Browser) -> Option<String> {
    let games: Vec<(String, BrowserView)> =
        SHELL.with(|shell| shell.borrow().games.iter().map(|(id, game)| (id.clone(), game.view.clone())).collect());
    games.into_iter().find(|(_, view)| same_browser(view, browser)).map(|(id, _)| id)
}

/// Se o browser é o de uma janela de jogo (e não a Biblioteca).
pub fn is_game_browser(browser: &Browser) -> bool {
    game_of(browser).is_some()
}

/// Se a janela do browser está em tela cheia pela opção "Abrir jogos em tela cheia".
pub fn is_auto_fullscreen(browser: &Browser) -> bool {
    game_of(browser).is_some_and(|id| SHELL.with(|shell| shell.borrow().games.get(&id).is_some_and(|g| g.auto_fullscreen)))
}

/// O usuário mexeu na tela cheia (F11): ela deixa de ser da opção.
pub fn clear_auto_fullscreen(browser: &Browser) {
    if let Some(id) = game_of(browser) {
        SHELL.with(|shell| {
            if let Some(game) = shell.borrow_mut().games.get_mut(&id) {
                game.auto_fullscreen = false;
            }
        });
    }
}

wrap_task! {
    pub struct OpenGameTask {
        game: Game,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            open_game(self.game.clone(), self.url.clone());
        }
    }
}
