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

Clique em **+ Adicionar jogo** na Biblioteca e informe a URL. O launcher lê a página (título, `og:image` e descrição) e abre o cadastro já preenchido para você revisar. O jogo é gravado em `~/.config/web-avenger/games.json`.

O catálogo padrão fica em `games.json` (embutido no binário). Também dá para editar `~/.config/web-avenger/games.json` à mão para adicionar ou sobrescrever jogos:

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

## Configurações

A aba **Configurações** da Biblioteca (salva em `~/.config/web-avenger/settings.json`) tem:

- **Abrir jogos em tela cheia** (ligada por padrão): a janela entra em tela cheia ao abrir um jogo e sai ao voltar para a Biblioteca (`F1`). `F11` continua alternando manualmente.
- **Tradução de input**: apresenta controles que não são de Xbox (ex.: Switch Pro Controller, DualSense) aos jogos como um controle de Xbox (`mapping: "standard"`, id XInput). Controles da Nintendo e da PlayStation, que o Chromium não enxerga direito com os drivers do kernel (`hid-nintendo`, `hid-playstation`), são lidos direto do `/dev/input/js*` pelo launcher (`src/native_pads.rs`) e traduzidos pelo código evdev de cada botão (`BTN_SOUTH` → A etc.).
- **Mapeamento manual**: para controles genéricos que os jogos não reconhecem. Escolha o controle, clique em **Mapear** e aperte cada botão pedido (ou **Pular**). O mapeamento é salvo por `vendor:product` e tem prioridade sobre o automático.
- **Botões de controles Nintendo**: por posição (B embaixo = A do Xbox) ou por rótulo (A = A).
- **Testar controle**: mostra ao vivo o que o Chromium vê e o que os jogos vão ver.

As mudanças valem para jogos abertos depois da alteração.

## Dados

O perfil do navegador (cache, IndexedDB, saves e cookies) fica em `~/.local/share/web-avenger/`.
