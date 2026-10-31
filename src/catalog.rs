use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

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
}

/// Caminho do catálogo do usuário: `~/.config/web-avenger/games.json`.
pub fn user_catalog_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("web-avenger").join("games.json"))
}

/// Carrega o catálogo embutido e mescla o catálogo do usuário por cima
/// (entradas com o mesmo `id` são substituídas, as novas são adicionadas).
pub fn load() -> Vec<Game> {
    let mut games: Vec<Game> =
        serde_json::from_str(DEFAULT_CATALOG).expect("games.json embutido é inválido");

    let Some(path) = user_catalog_path().filter(|path| path.exists()) else {
        return games;
    };

    let user_games = fs::read_to_string(&path)
        .map_err(|err| err.to_string())
        .and_then(|text| serde_json::from_str::<Vec<Game>>(&text).map_err(|err| err.to_string()));

    match user_games {
        Ok(user_games) => {
            for game in user_games {
                match games.iter_mut().find(|existing| existing.id == game.id) {
                    Some(existing) => *existing = game,
                    None => games.push(game),
                }
            }
        }
        Err(err) => eprintln!("[web-avenger] ignorando {}: {err}", path.display()),
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

/// Adiciona um jogo ao catálogo do usuário, gerando um `id` único.
pub fn add_user_game(mut game: Game) -> io::Result<Game> {
    let path = user_catalog_path()
        .ok_or_else(|| io::Error::other("diretório de configuração não encontrado"))?;

    // Um catálogo inválido não é sobrescrito: o usuário perderia os jogos dele.
    let mut user_games: Vec<Game> = match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|err| io::Error::other(format!("{} é inválido: {err}", path.display())))?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(err),
    };

    let taken: Vec<String> = load().into_iter().map(|game| game.id).collect();
    let base = slugify(&game.name);
    game.id = base.clone();
    let mut suffix = 2;
    while taken.contains(&game.id) {
        game.id = format!("{base}-{suffix}");
        suffix += 1;
    }

    user_games.push(game.clone());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, serde_json::to_string_pretty(&user_games)?)?;
    Ok(game)
}
