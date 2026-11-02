//! Atalho de um jogo na área de trabalho: roda este executável com `--game=<id>`, que
//! abre só a janela do jogo (ver `app.rs`). No Linux é um `.desktop`; no Windows, um
//! `.lnk` criado pelo PowerShell (`WScript.Shell`).
//!
//! O app ainda não tem ícone próprio: no Linux o atalho usa o ícone de jogo do tema e,
//! no Windows, o do executável.

use crate::catalog::Game;
use std::{
    io,
    path::{Path, PathBuf},
};

/// Argumento da linha de comando que abre um jogo.
pub const GAME_SWITCH: &str = "game";

/// Cria (ou substitui) o atalho do jogo na área de trabalho e devolve o caminho dele.
pub fn create(game: &Game) -> io::Result<PathBuf> {
    let desktop = dirs::desktop_dir().ok_or_else(|| io::Error::other(crate::i18n::t("shortcut.no_desktop")))?;
    let exe = std::env::current_exe()?;
    std::fs::create_dir_all(&desktop)?;
    write_shortcut(&desktop, &exe, game)
}

#[cfg(target_os = "linux")]
fn write_shortcut(desktop: &Path, exe: &Path, game: &Game) -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let path = desktop.join(format!("web-avenger-{}.desktop", game.id));
    std::fs::write(&path, desktop_entry(&exe.to_string_lossy(), game))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    // O GNOME só executa atalhos marcados como confiáveis; sem `gio`, o usuário confirma
    // na primeira vez.
    let _ = std::process::Command::new("gio")
        .arg("set")
        .arg(&path)
        .args(["metadata::trusted", "true"])
        .output();
    Ok(path)
}

#[cfg(windows)]
fn write_shortcut(desktop: &Path, exe: &Path, game: &Game) -> io::Result<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let path = desktop.join(format!("{}.lnk", lnk_name(&game.name)));
    // Os valores vão por variáveis de ambiente: nada de montar aspas dentro do script.
    let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:AVENGER_LNK); \
                  $s.TargetPath = $env:AVENGER_EXE; $s.Arguments = $env:AVENGER_ARGS; \
                  $s.WorkingDirectory = $env:AVENGER_DIR; $s.Save()";
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("AVENGER_LNK", &path)
        .env("AVENGER_EXE", exe)
        .env("AVENGER_ARGS", format!("--{GAME_SWITCH}={}", game.id))
        .env("AVENGER_DIR", exe.parent().unwrap_or(exe))
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!("o PowerShell não criou o atalho: {}", error.trim())));
    }
    Ok(path)
}

/// Conteúdo do `.desktop` que abre o jogo.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry(exe: &str, game: &Game) -> String {
    let name: String = game.name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={} --{GAME_SWITCH}={}\nIcon=applications-games\nCategories=Game;\n",
        exec_arg(exe),
        exec_arg(&game.id),
    )
}

/// Argumento do `Exec` de um `.desktop`: entre aspas se tiver caractere reservado, com
/// `"`, `` ` ``, `$` e `\` escapados; `%` vira `%%`; e o escape de string (`\` → `\\`)
/// por cima de tudo, como pede a especificação.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn exec_arg(arg: &str) -> String {
    const RESERVED: &[char] =
        &[' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`'];
    let quoted = if arg.contains(RESERVED) {
        let mut quoted = String::from("\"");
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('"');
        quoted
    } else {
        arg.to_string()
    };
    quoted.replace('%', "%%").replace('\\', "\\\\")
}

/// Nome de arquivo válido no Windows para o `.lnk`.
#[cfg_attr(not(windows), allow(dead_code))]
fn lnk_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if c.is_control() || r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
        .collect();
    let name = name.trim().trim_end_matches('.');
    if name.is_empty() { "Jogo".into() } else { name.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapa_argumentos_do_exec() {
        assert_eq!(exec_arg("/opt/web-avenger/web-avenger"), "/opt/web-avenger/web-avenger");
        assert_eq!(exec_arg("/home/a b/web-avenger"), r#""/home/a b/web-avenger""#);
        assert_eq!(exec_arg("/x/$y"), r#""/x/\\$y""#);
        assert_eq!(exec_arg("/x/100%"), "/x/100%%");
    }

    #[test]
    fn entrada_do_atalho() {
        let game: Game = serde_json::from_value(serde_json::json!({ "id": "polytrack", "name": "Poly\nTrack", "url": "x" })).unwrap();
        let entry = desktop_entry("/opt/wa/web-avenger", &game);
        assert!(entry.contains("Name=Poly Track\n"));
        assert!(entry.contains("Exec=/opt/wa/web-avenger --game=polytrack\n"));
    }

    #[test]
    fn nome_do_lnk() {
        assert_eq!(lnk_name("Jogo: o retorno?"), "Jogo_ o retorno_");
        assert_eq!(lnk_name("  ... "), "Jogo");
    }
}
