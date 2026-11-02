//! Traduções da interface: `locales/<código>.json`, embutidos no binário (ver `build.rs`).
//!
//! Cada arquivo é um JSON plano de chave → texto, com `"_name"` (nome do idioma no
//! seletor). Parâmetros vêm como `{nome}`. O pt-BR é a base: uma chave que falta no idioma
//! escolhido sai em pt-BR, então traduções parciais funcionam.

use crate::settings;
use serde_json::{Map, Value};

/// `(código, json)` de cada arquivo de `locales/`, em ordem alfabética.
const LOCALES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/locales.rs"));
/// Idioma base: completo, e usado quando falta uma chave.
pub const BASE: &str = "pt-BR";
/// Idioma do "Automático" quando não há tradução para o idioma do sistema.
const FALLBACK: &str = "en";

fn parse(code: &str) -> Map<String, Value> {
    LOCALES
        .iter()
        .find(|(lang, _)| *lang == code)
        .and_then(|(_, json)| serde_json::from_str(json).ok())
        .unwrap_or_default()
}

/// `(código, nome)` dos idiomas disponíveis, para o seletor das Configurações.
pub fn languages() -> Vec<(&'static str, String)> {
    LOCALES
        .iter()
        .map(|(code, _)| (*code, parse(code)["_name"].as_str().unwrap_or(code).to_string()))
        .collect()
}

/// O idioma configurado, se existir; senão o do sistema (`pt_BR.UTF-8`, `pt-PT` e `pt`
/// servem para `pt-BR`); senão inglês.
pub fn resolve(setting: Option<&str>) -> &'static str {
    resolve_with(setting, sys_locale::get_locale().as_deref())
}

fn resolve_with(setting: Option<&str>, system: Option<&str>) -> &'static str {
    setting.and_then(exact).or_else(|| system.and_then(closest)).unwrap_or(FALLBACK)
}

fn exact(code: &str) -> Option<&'static str> {
    LOCALES.iter().map(|(lang, _)| *lang).find(|lang| lang.eq_ignore_ascii_case(code))
}

fn closest(tag: &str) -> Option<&'static str> {
    let tag = tag.split(['.', '@']).next()?.replace('_', "-");
    let language = tag.split('-').next()?;
    exact(&tag).or_else(|| {
        LOCALES
            .iter()
            .map(|(lang, _)| *lang)
            .find(|lang| lang.split('-').next().is_some_and(|l| l.eq_ignore_ascii_case(language)))
    })
}

/// Os textos do idioma `code`, com o pt-BR por baixo para as chaves que faltam.
pub fn strings(code: &str) -> Map<String, Value> {
    let mut strings = parse(BASE);
    if code != BASE {
        strings.extend(parse(code));
    }
    strings
}

/// O idioma em uso agora (lido das configurações).
pub fn current() -> &'static str {
    if cfg!(test) {
        return BASE;
    }
    resolve(settings::load().language.as_deref())
}

/// O texto da chave no idioma em uso (ou a própria chave, se não existir).
pub fn t(key: &str) -> String {
    tf(key, &[])
}

/// Como `t`, trocando cada `{nome}` pelo valor correspondente.
pub fn tf(key: &str, vars: &[(&str, &str)]) -> String {
    fill(strings(current()).get(key).and_then(Value::as_str).unwrap_or(key), vars)
}

fn fill(text: &str, vars: &[(&str, &str)]) -> String {
    vars.iter().fold(text.to_string(), |text, (name, value)| text.replace(&format!("{{{name}}}"), value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn keys(code: &str) -> BTreeSet<String> {
        parse(code).keys().cloned().collect()
    }

    #[test]
    fn ingles_tem_as_mesmas_chaves_do_portugues() {
        let base = keys(BASE);
        let en = keys("en");
        let missing: Vec<_> = base.difference(&en).collect();
        let extra: Vec<_> = en.difference(&base).collect();
        assert!(missing.is_empty() && extra.is_empty(), "faltam em en: {missing:?}; sobram: {extra:?}");
    }

    /// Chaves usadas no código: `t("…")`/`tf("…")` no Rust e `t("…")`, `tn("…")` e
    /// `data-i18n*="…"` na Biblioteca. `tn` usa `chave.one` e `chave.other`.
    fn used_keys(source: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        for pattern in ["t(\"", "tf(\"", "tn(\"", "data-i18n=\"", "data-i18n-placeholder=\"", "data-i18n-title=\"", "data-i18n-aria-label=\""] {
            for (start, _) in source.match_indices(pattern) {
                let before = source[..start].chars().next_back();
                // Ignora chamadas como `split("` ou `at("` (o `t` faz parte de outro nome).
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') && !pattern.starts_with("data") {
                    continue;
                }
                let rest = &source[start + pattern.len()..];
                let Some(end) = rest.find('"') else { continue };
                let key = &rest[..end];
                if pattern == "tn(\"" {
                    found.insert(format!("{key}.one"));
                    found.insert(format!("{key}.other"));
                } else {
                    found.insert(key.to_string());
                }
            }
        }
        found
    }

    fn rust_sources(dir: &std::path::Path, out: &mut String) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") && !path.ends_with("i18n.rs") {
                out.push_str(&std::fs::read_to_string(&path).unwrap());
            }
        }
    }

    #[test]
    fn toda_chave_usada_existe_no_portugues() {
        let mut source = include_str!("../assets/library.html").to_string();
        rust_sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut source);
        let base = keys(BASE);
        let missing: Vec<_> = used_keys(&source).into_iter().filter(|key| !base.contains(key)).collect();
        assert!(missing.is_empty(), "chaves sem texto em pt-BR: {missing:?}");
    }

    #[test]
    fn escolhe_o_idioma() {
        assert_eq!(resolve_with(Some("en"), Some("pt_BR.UTF-8")), "en");
        assert_eq!(resolve_with(None, Some("pt_BR.UTF-8")), "pt-BR");
        assert_eq!(resolve_with(None, Some("pt-PT")), "pt-BR");
        assert_eq!(resolve_with(None, Some("en-US")), "en");
        assert_eq!(resolve_with(None, Some("ja-JP")), "en");
        assert_eq!(resolve_with(Some("xx"), Some("pt-BR")), "pt-BR");
        assert_eq!(resolve_with(None, None), "en");
    }

    #[test]
    fn troca_parametros_e_cai_no_portugues() {
        assert_eq!(fill("{a} e {b}, {a}", &[("a", "1"), ("b", "2")]), "1 e 2, 1");
        // Idioma sem arquivo: tudo em pt-BR.
        assert_eq!(strings("xx"), parse(BASE));
        assert_eq!(strings("en")["_name"], "English");
    }
}
