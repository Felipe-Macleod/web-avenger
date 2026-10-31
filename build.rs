fn main() {
    // libcef.so e os recursos do CEF ficam ao lado do executável (ver scripts/bundle.sh).
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");

    // `client_id` do app OAuth do itch.io: do ambiente ou do `.env`, embutido no binário
    // (o ambiente em tempo de execução ainda tem prioridade, ver `src/itch/mod.rs`).
    println!("cargo:rerun-if-env-changed=ITCH_CLIENT_ID");
    println!("cargo:rerun-if-changed=.env");
    let client_id = std::env::var("ITCH_CLIENT_ID").ok().or_else(|| env_file_value("ITCH_CLIENT_ID"));
    if let Some(client_id) = client_id.filter(|id| !id.is_empty()) {
        println!("cargo:rustc-env=ITCH_CLIENT_ID={client_id}");
    }
}

/// Valor de `NOME=valor` no `.env` da raiz (aceita aspas e comentários com `#`).
fn env_file_value(name: &str) -> Option<String> {
    let text = std::fs::read_to_string(".env").ok()?;
    text.lines().find_map(|line| {
        let (key, value) = line.trim().split_once('=')?;
        (key.trim() == name).then(|| value.trim().trim_matches(['"', '\'']).to_string())
    })
}
