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

/// Configurações do launcher, em `~/.config/web-avenger/settings.json`.
///
/// Lidas pelo processo de renderização a cada página carregada, então mudanças
/// valem para o próximo jogo aberto.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Apresenta controles não-Xbox aos jogos como um controle Xbox.
    pub input_translation: bool,
    pub button_layout: ButtonLayout,
    /// Mapeamentos manuais por controle (`"vendor:product"` → mapeamento).
    /// O formato é definido pelo tradutor em `assets/input-translation.js`.
    pub mappings: BTreeMap<String, serde_json::Value>,
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
