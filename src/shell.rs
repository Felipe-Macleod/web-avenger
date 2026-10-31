//! Qual browser a janela mostra: a Biblioteca (perfil principal) ou um jogo manual
//! (perfil próprio, ver `profiles`).
//!
//! No CEF o perfil é fixo por browser, então cada jogo manual tem um `BrowserView` com
//! o contexto dele. Todos ficam na janela e só um fica visível. O browser de um jogo
//! é criado na primeira vez que ele abre e reaproveitado depois: ao voltar para a
//! Biblioteca ele vai para uma página em branco e fica oculto, mas não fecha. No
//! estilo Chrome, fechar o último browser de um perfil encerra o app (e fechar o
//! browser de um view fora da janela derruba o CEF). Tudo roda na thread de UI do
//! processo principal.

use crate::{app::AvengerBrowserViewDelegate, catalog::Game, client::AvengerClient, profiles};
use cef::*;
use std::{cell::RefCell, collections::HashMap};

struct Shell {
    window: Window,
    library: BrowserView,
    /// Browser de cada jogo manual aberto nesta sessão (id do jogo → view).
    games: HashMap<String, BrowserView>,
    /// Jogo visível agora; `None` = Biblioteca.
    current: Option<String>,
}

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
}

pub fn init(window: Window, library: BrowserView) {
    SHELL.with(|shell| {
        *shell.borrow_mut() = Some(Shell { window, library, games: HashMap::new(), current: None })
    });
}

pub fn clear() {
    SHELL.with(|shell| shell.borrow_mut().take());
}

/// O browser visível na janela.
pub fn active_view() -> Option<BrowserView> {
    SHELL.with(|shell| {
        let shell = shell.borrow();
        let shell = shell.as_ref()?;
        let game = shell.current.as_ref().and_then(|id| shell.games.get(id));
        Some(game.unwrap_or(&shell.library).clone())
    })
}

fn same_browser(view: &BrowserView, browser: &Browser) -> bool {
    view.browser().is_some_and(|own| own.is_same(Some(&mut browser.clone())) != 0)
}

fn load(view: &BrowserView, url: &str) {
    if let Some(frame) = view.browser().and_then(|browser| browser.main_frame()) {
        frame.load_url(Some(&CefString::from(url)));
    }
}

/// Mostra `show` e esconde os outros views da janela.
fn show_only(window: &Window, show: &BrowserView, others: &[BrowserView]) {
    show.set_visible(1);
    for view in others {
        view.set_visible(0);
    }
    window.layout();
    show.request_focus();
}

/// Abre um jogo manual no perfil dele, no lugar do browser visível.
pub fn open_game(game: Game, url: String) {
    // Já aberto nesta sessão: reaproveita o browser (o perfil já está inicializado).
    let existing = SHELL.with(|shell| shell.borrow().as_ref().and_then(|shell| shell.games.get(&game.id).cloned()));
    if let Some(view) = existing {
        load(&view, &url);
        return present(&game.id, &view);
    }
    let id = game.id.clone();
    profiles::with_context(&game, Box::new(move |context| create_game_view(&id, context, &url)));
}

fn create_game_view(id: &str, mut context: RequestContext, url: &str) {
    let mut client = AvengerClient::new();
    let mut delegate = AvengerBrowserViewDelegate::new();
    let Some(view) = browser_view_create(
        Some(&mut client),
        Some(&CefString::from(url)),
        Some(&BrowserSettings::default()),
        None,
        Some(&mut context),
        Some(&mut delegate),
    ) else {
        eprintln!("[web-avenger] não foi possível abrir {url}");
        return;
    };
    // O estado é atualizado antes de mexer na janela, e sem manter o `RefCell`
    // emprestado: mexer nos views dispara callbacks (ex.: `on_before_browse`) que o consultam.
    let Some(window) = SHELL.with(|shell| {
        let mut shell = shell.borrow_mut();
        let shell = shell.as_mut()?;
        shell.games.insert(id.to_string(), view.clone());
        Some(shell.window.clone())
    }) else {
        return;
    };
    window.add_child_view(Some(&mut View::from(&view)));
    present(id, &view);
}

/// Torna o jogo `id` o view visível.
fn present(id: &str, view: &BrowserView) {
    let Some((window, others)) = SHELL.with(|shell| {
        let mut shell = shell.borrow_mut();
        let shell = shell.as_mut()?;
        shell.current = Some(id.to_string());
        let others = shell
            .games
            .iter()
            .filter(|(other, _)| other.as_str() != id)
            .map(|(_, view)| view.clone())
            .chain([shell.library.clone()])
            .collect::<Vec<_>>();
        Some((shell.window.clone(), others))
    }) else {
        return;
    };
    show_only(&window, view, &others);
}

/// Volta para a Biblioteca; o jogo vai para uma página em branco (para o som e a CPU).
pub fn show_library() {
    let Some((window, library, game)) = SHELL.with(|shell| {
        let mut shell = shell.borrow_mut();
        let shell = shell.as_mut()?;
        let id = shell.current.take()?;
        Some((shell.window.clone(), shell.library.clone(), shell.games.get(&id).cloned()?))
    }) else {
        return;
    };
    show_only(&window, &library, std::slice::from_ref(&game));
    load(&game, "about:blank");
    // Recarrega para atualizar "Recentes" (e passar pela saída da tela cheia).
    load(&library, crate::library_url());
}

/// Se o browser é o que está visível na janela.
pub fn is_active(browser: &Browser) -> bool {
    active_view().is_none_or(|view| same_browser(&view, browser))
}

/// Se o browser é o de um jogo manual (e não a Biblioteca).
pub fn is_game_browser(browser: &Browser) -> bool {
    let games: Vec<BrowserView> =
        SHELL.with(|shell| shell.borrow().as_ref().map(|shell| shell.games.values().cloned().collect()).unwrap_or_default());
    games.iter().any(|view| same_browser(view, browser))
}

/// Fechamento da janela: fecha os jogos e pede à Biblioteca para fechar. Retorna se a
/// janela já pode fechar (`try_close_browser` da Biblioteca).
pub fn close_all() -> i32 {
    let Some((library, games)) = SHELL.with(|shell| {
        let shell = shell.borrow();
        let shell = shell.as_ref()?;
        Some((shell.library.clone(), shell.games.values().cloned().collect::<Vec<_>>()))
    }) else {
        return 1;
    };
    for game in games {
        if let Some(host) = game.browser().and_then(|browser| browser.host()) {
            host.close_browser(1);
        }
    }
    match library.browser().and_then(|browser| browser.host()) {
        Some(host) => host.try_close_browser(),
        None => 1,
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

wrap_task! {
    pub struct ShowLibraryTask;

    impl Task {
        fn execute(&self) {
            show_library();
        }
    }
}
