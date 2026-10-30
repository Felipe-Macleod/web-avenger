use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

const DEFAULT_CATALOG: &str = include_str!("../games.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
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
