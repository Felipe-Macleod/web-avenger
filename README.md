# Web Avenger

Agregador standalone de jogos WebAssembly, escrito em Rust. Os jogos rodam **dentro do app**, num Chromium embutido via [CEF](https://bitbucket.org/chromiumembedded/cef) (crate [`cef`](https://github.com/tauri-apps/cef-rs)). O WebGPU já vem habilitado.

## Build

Requisitos: Rust 1.85+, `cmake` e `ninja`. O primeiro build baixa o CEF (~1 GB descompactado).

```sh
./scripts/bundle.sh          # cargo build --release + monta dist/web-avenger/
./dist/web-avenger/web-avenger
```

A pasta `dist/web-avenger/` é autocontida (~370 MB): basta copiá-la para outra máquina Linux x86_64.

Para não baixar o CEF a cada `cargo clean`, exporte uma cópia compartilhada:

```sh
export CEF_PATH="$HOME/.local/share/cef"   # o build baixa para lá se não existir
```

## Atalhos

| Tecla | Ação |
|-------|------|
| `F1`  | Voltar para a Biblioteca |
| `F11` | Tela cheia |
| `F5`  | Recarregar (ignorando cache) |

No Wayland o app roda via XWayland (`--ozone-platform=x11`), porque o CEF em Wayland nativo não acompanha os redimensionamentos do compositor.

Para depurar, use `./web-avenger --remote-debugging-port=9222` e abra `chrome://inspect` em outro Chrome.

## Adicionando jogos

O catálogo padrão fica em `games.json` (embutido no binário). Para adicionar ou sobrescrever jogos sem recompilar, crie `~/.config/web-avenger/games.json`:

```json
[
  {
    "id": "meu-jogo",
    "name": "Meu Jogo",
    "description": "Opcional",
    "url": "https://exemplo.com/jogo/",
    "thumbnail": "https://exemplo.com/capa.jpg",
    "tags": ["wasm"]
  }
]
```

Entradas com o mesmo `id` de um jogo embutido substituem a original.

## Dados

O perfil do navegador (cache, IndexedDB, saves e cookies) fica em `~/.local/share/web-avenger/`.
