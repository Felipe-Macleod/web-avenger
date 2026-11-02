fn main() {
    // libcef.so e os recursos do CEF ficam ao lado do executável (ver scripts/bundle.sh).
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");

    // O CEF exige o manifesto de compatibilidade com o Windows 10+ no executável; sem ele
    // o Chromium enxerga uma versão antiga do Windows. O padrão do `embed-manifest` já
    // declara Windows 10/11, DPI por monitor (v2) e caminhos longos.
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest::embed_manifest(embed_manifest::new_manifest("WebAvenger"))
            .expect("falha ao embutir o manifesto do Windows");
    }

    // `client_id` do app OAuth do itch.io: do ambiente ou do `.env`, embutido no binário
    // (o ambiente em tempo de execução ainda tem prioridade, ver `src/itch/mod.rs`).
    println!("cargo:rerun-if-env-changed=ITCH_CLIENT_ID");
    println!("cargo:rerun-if-changed=.env");
    let client_id = std::env::var("ITCH_CLIENT_ID").ok().or_else(|| env_file_value("ITCH_CLIENT_ID"));
    if let Some(client_id) = client_id.filter(|id| !id.is_empty()) {
        println!("cargo:rustc-env=ITCH_CLIENT_ID={client_id}");
    }

    // Catálogo padrão (`games.json` da raiz, não versionado): embutido no binário se existir;
    // sem ele, o catálogo padrão fica vazio (ver `src/catalog.rs`).
    println!("cargo:rerun-if-changed=games.json");
    let catalog = std::fs::read_to_string("games.json").unwrap_or_else(|_| "[]".into());
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR não definido");
    std::fs::write(std::path::Path::new(&out_dir).join("games.json"), catalog)
        .expect("falha ao gravar o catálogo padrão");

    embed_locales(&out_dir);
}

/// Embute as traduções de `locales/*.json` (ver `src/i18n.rs`): gera `locales.rs` com a
/// lista `(código, json)`. Um arquivo que não é JSON (ou sem `_name`) faz o build falhar.
fn embed_locales(out_dir: &str) {
    println!("cargo:rerun-if-changed=locales");
    let dir = std::path::Path::new("locales").canonicalize().expect("pasta locales/ não encontrada");
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("falha ao ler locales/")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort();
    let mut code = String::from("&[\n");
    for path in entries {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let json: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} não é um JSON válido: {e}", path.display()));
        assert!(json["_name"].is_string(), "{} não tem \"_name\"", path.display());
        let lang = path.file_stem().unwrap().to_string_lossy();
        code.push_str(&format!("    ({lang:?}, include_str!({:?})),\n", path.to_string_lossy()));
    }
    code.push(']');
    std::fs::write(std::path::Path::new(out_dir).join("locales.rs"), code).expect("falha ao gravar locales.rs");
}

/// Valor de `NOME=valor` no `.env` da raiz (aceita aspas e comentários com `#`).
fn env_file_value(name: &str) -> Option<String> {
    let text = std::fs::read_to_string(".env").ok()?;
    text.lines().find_map(|line| {
        let (key, value) = line.trim().split_once('=')?;
        (key.trim() == name).then(|| value.trim().trim_matches(['"', '\'']).to_string())
    })
}
