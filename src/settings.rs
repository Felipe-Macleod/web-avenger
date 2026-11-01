use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::PathBuf};

/// Como os botões de face de controles Nintendo viram botões de Xbox.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonLayout {
    /// Pela posição física: o botão de baixo (B no Nintendo) vira o A do Xbox.
    #[default]
    Position,
    /// Pelo rótulo impresso: o A do Nintendo vira o A do Xbox.
    Label,
}

/// Feed RSS/Atom cadastrado na aba Explorar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feed {
    pub name: String,
    pub url: String,
}

/// Configurações do launcher, em `~/.config/web-avenger/settings.json`.
///
/// Lidas pelo processo de renderização a cada página carregada, então mudanças
/// valem para o próximo jogo aberto.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Coloca a janela em tela cheia ao abrir um jogo pela Biblioteca.
    pub fullscreen_games: bool,
    /// Faz players Unity WebGL ocuparem a janela inteira (mantendo a proporção).
    pub maximize_player: bool,
    /// Mostra jogos com tags adultas (NSFW, Porn, Hentai...); desligado por padrão.
    /// O filtro em si fica na Biblioteca (`assets/library.html`).
    pub show_adult: bool,
    /// Apresenta controles não-Xbox aos jogos como um controle Xbox.
    pub input_translation: bool,
    pub button_layout: ButtonLayout,
    /// Mapeamentos manuais por controle (`"vendor:product"` → mapeamento).
    /// O formato é definido pelo tradutor em `assets/input-translation.js`.
    pub mappings: BTreeMap<String, serde_json::Value>,
    /// Feeds do Explorar, na ordem do cadastro.
    pub feeds: Vec<Feed>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fullscreen_games: true,
            maximize_player: true,
            show_adult: false,
            input_translation: false,
            button_layout: ButtonLayout::default(),
            mappings: BTreeMap::new(),
            feeds: Vec::new(),
        }
    }
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("web-avenger").join("settings.json"))
}

pub fn load() -> Settings {
    path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> io::Result<()> {
    let path = path().ok_or_else(|| io::Error::other("diretório de configuração não encontrado"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(settings)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuracao_antiga_sem_feeds() {
        let settings: Settings = serde_json::from_str(r#"{"show_adult": true}"#).unwrap();
        assert!(settings.show_adult && settings.feeds.is_empty() && settings.fullscreen_games);
    }
}
