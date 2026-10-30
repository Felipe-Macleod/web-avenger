use crate::catalog::Game;
use base64::{Engine, engine::general_purpose::STANDARD};

const TEMPLATE: &str = include_str!("../assets/library.html");

/// Gera a página da Biblioteca como uma URL `data:` autocontida.
///
/// Não usa os utilitários do CEF porque roda antes de `cef::initialize`.
pub fn data_url(games: &[Game]) -> String {
    let json = serde_json::to_string(games)
        .expect("falha ao serializar o catálogo")
        // Impede que um `</script>` no catálogo feche a tag de script.
        .replace("</", "<\\/");
    let html = TEMPLATE.replace("/*GAMES*/[]", &json);
    format!("data:text/html;charset=utf-8;base64,{}", STANDARD.encode(html))
}
