use crate::catalog::Game;
use std::{fs, io, path::PathBuf};

const TEMPLATE: &str = include_str!("../assets/library.html");
const FILE_NAME: &str = "library.html";

/// A Biblioteca é servida de um arquivo (e não de `data:`) porque `file://` é um
/// contexto seguro, o que libera a Gamepad API para o testador de controle.
fn path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("web-avenger")
        .join(FILE_NAME)
}

/// Gera a página da Biblioteca e devolve a URL `file://` dela.
pub fn write(games: &[Game]) -> io::Result<String> {
    let json = serde_json::to_string(games)
        .map_err(io::Error::other)?
        // Impede que um `</script>` no catálogo feche a tag de script.
        .replace("</", "<\\/");
    let html = TEMPLATE.replace("/*GAMES*/[]", &json);

    let path = path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, html)?;
    Ok(format!("file://{}", path.to_string_lossy().replace(' ', "%20")))
}

/// Se a URL de um frame é a Biblioteca (usado no processo de renderização).
pub fn is_library_url(url: &str) -> bool {
    url.starts_with("file://") && url.ends_with(&format!("/web-avenger/{FILE_NAME}"))
}
