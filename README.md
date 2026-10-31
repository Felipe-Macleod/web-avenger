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

A Biblioteca tem uma sidebar com filtros (Todos os jogos, Recentes, Fontes e Categorias pelas tags), busca e alternância entre grade e lista.

Clique em **+ Adicionar** na Biblioteca e informe a URL. O launcher lê a página (título, `og:image` e descrição) e abre o cadastro já preenchido para você revisar.

Páginas do **itch.io** são reconhecidas sem chave de API: o nome, a capa e as tags vêm do `data.json` público da página, e a URL salva é a do jogo HTML5 (`html-classic.itch.zone/html/<upload>/…`, o mesmo que o "Run game" carrega), com a página do itch guardada como `homepage`. Jogos só para download não têm versão de navegador e ficam com a URL da página. O jogo é gravado em `~/.config/web-avenger/games.json`.

Para alterar um jogo, clique com o botão direito no card (ou use a tecla Menu / Shift+F10) e escolha **Editar jogo**. No modal de edição, **Remover jogo** (clique duas vezes para confirmar) tira o jogo da Biblioteca; jogos do catálogo padrão ficam marcados com `"removed": true` no seu `games.json`. Editar um jogo do catálogo padrão grava uma cópia em `~/.config/web-avenger/games.json`, que prevalece sobre a original.

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

O item **Configurações** da sidebar (salva em `~/.config/web-avenger/settings.json`) tem:

- **Abrir jogos em tela cheia** (ligada por padrão): a janela entra em tela cheia ao abrir um jogo e sai ao voltar para a Biblioteca (`F1`). `F11` continua alternando manualmente.
- **Maximizar o player do jogo** (ligada por padrão): players Unity WebGL (template padrão do Unity 2020+ e do UnityLoader antigo) ocupam a janela inteira mantendo a proporção, sem o rodapé (`assets/player-fit.js`).
- **Tradução de input**: apresenta controles que não são de Xbox (ex.: Switch Pro Controller, DualSense) aos jogos como um controle de Xbox (`mapping: "standard"`, id XInput). Controles da Nintendo e da PlayStation, que o Chromium não enxerga direito com os drivers do kernel (`hid-nintendo`, `hid-playstation`), são lidos direto do `/dev/input/js*` pelo launcher (`src/native_pads.rs`) e traduzidos pelo código evdev de cada botão (`BTN_SOUTH` → A etc.).
- **Mapeamento manual**: para controles genéricos que os jogos não reconhecem. Escolha o controle, clique em **Mapear** e aperte cada botão pedido (ou **Pular**). O mapeamento é salvo por `vendor:product` e tem prioridade sobre o automático.
- **Botões de controles Nintendo**: por posição (B embaixo = A do Xbox) ou por rótulo (A = A).
- **Testar controle**: mostra ao vivo o que o Chromium vê e o que os jogos vão ver.

As mudanças valem para jogos abertos depois da alteração.

## Áudio AAC

As builds do CEF não trazem codecs proprietários, então o Chromium embutido não decodifica AAC — o formato que o Unity WebGL usa para áudio comprimido (jogos ficariam mudos). O launcher intercepta `decodeAudioData` para buffers MP4/M4A (`assets/audio-fallback.js`) e decodifica em Rust com o [Symphonia](https://github.com/pdeljanov/Symphonia) (`src/audio.rs`). MP3, Ogg/Vorbis e Opus continuam com o decodificador do Chromium.

## Dados

O perfil do navegador (cache, IndexedDB, saves e cookies) fica em `~/.local/share/web-avenger/`.
