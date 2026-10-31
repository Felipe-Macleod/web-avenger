use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const DEFAULT_CATALOG: &str = include_str!("../games.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    /// Gerado a partir do nome quando o jogo é cadastrado pela Biblioteca.
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub url: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Marca um jogo do catálogo embutido como removido pelo usuário (não aparece
    /// na Biblioteca). Jogos do próprio usuário são apagados de verdade.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
    /// De onde o jogo veio (ex.: `"itch.io"`); ausente = cadastro manual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Id do jogo no itch.io, para a sincronização não importar o mesmo jogo duas vezes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub itch_id: Option<u64>,
}

/// Origem dos jogos importados do itch.io (ver `src/itch/sync.rs`).
pub const ITCH_SOURCE: &str = "itch.io";

/// Caminho do catálogo do usuário: `~/.config/web-avenger/games.json` (jogos manuais
/// e edições/remoções de jogos do catálogo embutido).
pub fn user_catalog_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("web-avenger").join("games.json"))
}

/// Jogos importados do itch.io: `~/.config/web-avenger/itch-games.json`. Edições e
/// remoções desses jogos ficam no próprio arquivo (removidos com `removed: true`, para
/// a sincronização não trazê-los de volta).
pub fn itch_catalog_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("web-avenger").join("itch-games.json"))
}

fn builtin_games() -> Vec<Game> {
    serde_json::from_str(DEFAULT_CATALOG).expect("games.json embutido é inválido")
}

/// Jogos da Biblioteca (sem os removidos).
pub fn load() -> Vec<Game> {
    load_all().into_iter().filter(|game| !game.removed).collect()
}

/// Mescla, nesta ordem, o catálogo embutido, os jogos do itch.io e o catálogo do
/// usuário (entradas com o mesmo `id` são substituídas, as novas são adicionadas).
fn load_all() -> Vec<Game> {
    let mut games = builtin_games();
    for path in [itch_catalog_path(), user_catalog_path()].into_iter().flatten() {
        let layer = match read_games(&path) {
            Ok(layer) => layer,
            Err(err) => {
                eprintln!("[web-avenger] ignorando {}: {err}", path.display());
                continue;
            }
        };
        for game in layer {
            match games.iter_mut().find(|existing| existing.id == game.id) {
                Some(existing) => *existing = game,
                None => games.push(game),
            }
        }
    }
    games
}

/// Transforma o nome em um id: ASCII minúsculo, separado por hífens.
fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() { "jogo".into() } else { slug.into() }
}

/// Lê um arquivo de jogos (vazio se não existir).
fn read_games(path: &Path) -> io::Result<Vec<Game>> {
    // Um arquivo inválido não é sobrescrito: o usuário perderia os jogos dele.
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|err| io::Error::other(format!("{} é inválido: {err}", path.display()))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

fn write_games(path: &Path, games: &[Game]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(games)?)
}

fn config_path(path: Option<PathBuf>) -> io::Result<PathBuf> {
    path.ok_or_else(|| io::Error::other("diretório de configuração não encontrado"))
}

/// Gera `id`s únicos (pelo nome) para jogos novos; `taken` inclui os removidos, cujos
/// ids não são reaproveitados.
fn assign_ids(games: Vec<Game>) -> Vec<Game> {
    let mut taken: Vec<String> = load_all().into_iter().map(|game| game.id).collect();
    games
        .into_iter()
        .map(|mut game| {
            let base = slugify(&game.name);
            game.id = base.clone();
            let mut suffix = 2;
            while taken.contains(&game.id) {
                game.id = format!("{base}-{suffix}");
                suffix += 1;
            }
            taken.push(game.id.clone());
            game
        })
        .collect()
}

/// Adiciona um jogo manual ao catálogo do usuário, gerando um `id` único.
pub fn add_user_game(game: Game) -> io::Result<Game> {
    let path = config_path(user_catalog_path())?;
    let mut user_games = read_games(&path)?;
    let game = assign_ids(vec![game]).remove(0);
    user_games.push(game.clone());
    write_games(&path, &user_games)?;
    Ok(game)
}

/// Adiciona jogos importados do itch.io ao arquivo deles (uma única gravação).
pub fn add_itch_games(games: Vec<Game>) -> io::Result<Vec<Game>> {
    let path = config_path(itch_catalog_path())?;
    let mut itch_games = read_games(&path)?;
    let added = assign_ids(games);
    itch_games.extend(added.iter().cloned());
    write_games(&path, &itch_games)?;
    Ok(added)
}

/// Move para `itch-games.json` os jogos do itch.io que versões anteriores gravavam no
/// `games.json`. Grava primeiro o arquivo do itch.io: se algo falhar no meio, o jogo
/// fica duplicado (o do usuário prevalece), nunca perdido.
pub fn migrate_itch_games() -> io::Result<usize> {
    let user_path = config_path(user_catalog_path())?;
    let itch_path = config_path(itch_catalog_path())?;
    let mut user_games = read_games(&user_path)?;
    let (itch, manual): (Vec<Game>, Vec<Game>) = user_games
        .drain(..)
        .partition(|game| game.source.as_deref() == Some(ITCH_SOURCE));
    if itch.is_empty() {
        return Ok(0);
    }
    let mut itch_games = read_games(&itch_path)?;
    let moved = itch.len();
    for game in itch {
        match itch_games.iter_mut().find(|existing| existing.id == game.id) {
            Some(existing) => *existing = game,
            None => itch_games.push(game),
        }
    }
    write_games(&itch_path, &itch_games)?;
    write_games(&user_path, &manual)?;
    Ok(moved)
}

/// Páginas (`homepage`, ou a própria `url`) de todos os jogos, inclusive removidos.
pub fn load_all_pages() -> Vec<String> {
    load_all().into_iter().map(|game| game.homepage.unwrap_or(game.url)).collect()
}

/// Ids do itch.io já no catálogo, inclusive de jogos removidos (que não voltam).
pub fn known_itch_ids() -> std::collections::HashSet<u64> {
    load_all().into_iter().filter_map(|game| game.itch_id).collect()
}

/// Se o jogo vive no arquivo do itch.io.
fn in_itch_catalog(id: &str) -> io::Result<bool> {
    let path = config_path(itch_catalog_path())?;
    Ok(read_games(&path)?.iter().any(|game| game.id == id))
}

/// Grava `game` no arquivo indicado, substituindo a entrada com o mesmo `id`.
fn upsert(path: &Path, game: Game) -> io::Result<()> {
    let mut games = read_games(path)?;
    match games.iter_mut().find(|existing| existing.id == game.id) {
        Some(existing) => *existing = game,
        None => games.push(game),
    }
    write_games(path, &games)
}

/// Salva a edição de um jogo. Jogos do itch.io são editados no arquivo deles; um jogo
/// do catálogo embutido ganha uma cópia no catálogo do usuário, que prevalece sobre a
/// original (mesmo `id`).
pub fn update_user_game(mut game: Game) -> io::Result<Game> {
    if game.id.is_empty() {
        return Err(io::Error::other("jogo sem id"));
    }
    // O formulário de edição não conhece a origem: preserva a do jogo atual.
    if let Some(current) = load_all().into_iter().find(|existing| existing.id == game.id) {
        game.source = current.source;
        game.itch_id = current.itch_id;
    }
    game.removed = false;
    let path = if in_itch_catalog(&game.id)? { itch_catalog_path() } else { user_catalog_path() };
    upsert(&config_path(path)?, game.clone())?;
    Ok(game)
}

/// Remove um jogo da Biblioteca.
pub fn remove_user_game(id: &str) -> io::Result<()> {
    let Some(mut game) = load_all().into_iter().find(|game| game.id == id) else {
        return Err(io::Error::other(format!("jogo `{id}` não encontrado")));
    };

    if in_itch_catalog(id)? {
        // Fica marcado no arquivo do itch.io para a sincronização não trazê-lo de volta.
        game.removed = true;
        upsert(&config_path(itch_catalog_path())?, game)
    } else if builtin_games().iter().any(|builtin| builtin.id == id) {
        // O catálogo embutido não muda: fica uma marca de removido no do usuário.
        game.removed = true;
        upsert(&config_path(user_catalog_path())?, game)
    } else {
        let path = config_path(user_catalog_path())?;
        let mut user_games = read_games(&path)?;
        user_games.retain(|existing| existing.id != id);
        write_games(&path, &user_games)
    }
}
