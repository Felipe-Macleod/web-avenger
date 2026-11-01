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

### Windows

Requisitos: Rust com o toolchain MSVC (`rustup`), Visual Studio 2022 com a carga "Desenvolvimento para desktop com C++", `cmake` e `ninja` (ex.: `choco install cmake ninja`).

```powershell
.\scripts\bundle.ps1                 # cargo build --release + monta dist\web-avenger\
.\dist\web-avenger\web-avenger.exe
```

A pasta `dist\web-avenger\` é autocontida: basta copiá-la para outra máquina Windows x64. Para compartilhar o CEF entre builds, defina `$env:CEF_PATH`. No build de release o app não abre janela de console; para ver os logs, rode o build de debug (`cargo run`).

## Atalhos

| Tecla | Ação |
|-------|------|
| `F1`  | Voltar para a Biblioteca (na janela dela; nas janelas de jogo a tecla vai para o jogo) |
| `F11` | Tela cheia |
| `F5`  | Recarregar (ignorando cache) |

No Linux com Wayland o app roda via XWayland (`--ozone-platform=x11`), porque o CEF em Wayland nativo não acompanha os redimensionamentos do compositor.

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

## Compartilhar jogos

No menu do card (botão direito), **Compartilhar (copiar link)** copia um link como `webavenger://add?v=2&m=…`. O link carrega o próprio cadastro do jogo (nome, URL, página, descrição, capa `http(s)` e tags, num array JSON comprimido com deflate e codificado em base64url), então não depende de servidor. Links `v=1` (JSON sem compressão) continuam sendo aceitos. Quem recebe cola o link no campo de URL de **+ Adicionar** e o cadastro já aparece preenchido para revisar; nada é gravado antes de clicar em **Adicionar**. Jogos importados do itch.io viram jogos manuais para quem recebe.

## Tempo de jogo e ordenação

O launcher mede o tempo que cada jogo fica aberto (de abrir a janela do jogo até fechá-la; vários jogos abertos contam juntos) e a última vez jogado. Os cards mostram, por exemplo, "3 h 12 min · há 2 dias"; **Recentes** na sidebar lista os jogos já jogados, do mais recente ao mais antigo. Os dados ficam em `~/.local/share/web-avenger/playtime.json` (gravados também a cada minuto, para não perder tempo se o app cair).

O seletor na barra superior ordena a Biblioteca por **Recentes** (último jogado ou recém adicionado), **Nome** ou **Tempo de jogo**.

## Configurações

O item **Configurações** da sidebar (salva em `~/.config/web-avenger/settings.json`) tem:

- **Abrir jogos em tela cheia** (ligada por padrão): a janela do jogo já abre em tela cheia. `F11` continua alternando manualmente.
- **Maximizar o player do jogo** (ligada por padrão): players Unity WebGL (template padrão do Unity 2020+ e do UnityLoader antigo) ocupam a janela inteira mantendo a proporção, sem o rodapé; jogos do DMM GAMES / FANZA GAMES também: o jogo é ampliado sem o resto da página, usando o enquadramento mais próximo de 16:9 entre só o canvas do jogo (recortado do iframe) e o iframe inteiro (ex.: com a barra da loja), e o jogo recebe um `devicePixelRatio` multiplicado pela escala, para motores como Unity e Cocos renderizarem na resolução da tela em vez de esticar a imagem (`assets/player-fit.js`).
- **Mostrar jogos +18** (desligada por padrão): desligada, esconde da Biblioteca (grade, contagens, categorias e Recentes) os jogos com as tags `nsfw`, `porn`, `hentai`, `adult`, `erotic` ou `18+`.
- **Tradução de input**: apresenta controles que não são de Xbox (ex.: Switch Pro Controller, DualSense) aos jogos como um controle de Xbox (`mapping: "standard"`, id XInput). No Linux, controles da Nintendo e da PlayStation, que o Chromium não enxerga direito com os drivers do kernel (`hid-nintendo`, `hid-playstation`), são lidos direto do `/dev/input/js*` pelo launcher (`src/native_pads.rs`) e traduzidos pelo código evdev de cada botão (`BTN_SOUTH` → A etc.). No Windows o próprio Chromium já lê esses controles.
- **Mapeamento manual**: para controles genéricos que os jogos não reconhecem. Escolha o controle, clique em **Mapear** e aperte cada botão pedido (ou **Pular**). O mapeamento é salvo por `vendor:product` e tem prioridade sobre o automático.
- **Botões de controles Nintendo**: por posição (B embaixo = A do Xbox) ou por rótulo (A = A).
- **Testar controle**: mostra ao vivo o que o Chromium vê e o que os jogos vão ver.

As mudanças valem para jogos abertos depois da alteração.

## itch.io (Apps conectados)

Em **Configurações → Apps conectados**, o botão **Conectar** liga sua conta do itch.io por OAuth (authorization code + PKCE, sem client secret) e **Sincronizar** importa os jogos **de navegador** que você comprou ou tem nas suas coleções. Eles aparecem em **Fontes → itch.io** na sidebar. Sincronizar de novo só acrescenta jogos novos: não duplica, não desfaz suas edições e não traz de volta jogos que você removeu.

### Configuração (uma vez)

1. Registre um app OAuth em <https://itch.io/user/settings/oauth-apps> com a redirect URI exata:
   ```
   http://127.0.0.1/web-avenger/oauth/callback
   ```
   Nada escuta nesse endereço: o launcher intercepta o retorno dentro da própria janela.
2. Defina o `client_id` gerado em `ITCH_CLIENT_ID`, de um destes jeitos:
   - no arquivo `.env` na raiz do projeto (veja `.env.example`; o `.env` não é versionado) — o valor é **embutido no binário** no `cargo build`/`scripts/bundle.sh`;
   - ou no ambiente ao rodar o app (`ITCH_CLIENT_ID=… ./web-avenger`), que tem prioridade sobre o embutido.

### Onde fica o token

- O login acontece dentro do app, então a sessão do itch.io fica no perfil (`~/.local/share/web-avenger/`) — isso também libera jogos pagos de navegador na sincronização.
- O token OAuth fica em `~/.config/web-avenger/itch-token.enc` (permissão `0600`), **cifrado com ChaCha20-Poly1305**. A chave de 32 bytes fica no **chaveiro do sistema** (Secret Service: gnome-keyring, KWallet…; no Windows, o Gerenciador de Credenciais), como `web-avenger` / `itch-token-key`. Sem chaveiro disponível a conexão é recusada.
- O token só é lido pelo processo principal do launcher; nunca chega às páginas.
- Os jogos importados ficam em um arquivo só deles, `~/.config/web-avenger/itch-games.json` (o `games.json` fica com os jogos manuais). Editar ou remover um jogo importado altera esse arquivo; os removidos ficam marcados com `"removed": true` para a sincronização não trazê-los de volta.
- Os jogos comprados vêm da API (`/profile/owned-keys`). Os jogos de cada coleção são lidos da página da coleção (`itch.io/c/<id>`) com a sessão do app, porque o endpoint da API para isso exige o escopo `collection:view`, que o OAuth do itch.io não concede. Os jogos de navegador são reconhecidos pelo tipo do projeto (`html`) ou pelo selo "Play in browser".
- **Desconectar** revoga o token no itch.io e apaga o arquivo e a chave do chaveiro. Os jogos importados continuam na Biblioteca.

## DMM GAMES / FANZA GAMES (Apps conectados)

O DMM não tem API pública nem OAuth. Em **Configurações → Apps conectados → DMM GAMES / FANZA GAMES**, **Conectar** abre o login do próprio site dentro do app: primeiro o do DMM, depois o do FANZA, que tem sessão separada. Com a sessão ativa, os dois passam direto. A sessão fica nos cookies do perfil principal; nenhuma senha passa pelo launcher.

**Sincronizar** lê **マイゲーム** (Meus jogos, `library.games.dmm.com`) e importa os jogos de navegador para PC, com a URL de jogar (`play.games.dmm.com/game/<jogo>` ou `play.games.dmm.co.jp/game/<jogo>`), a página da loja, a capa, a descrição e o gênero. Jogos do DMM GAMES PLAYER, de celular ou de sites parceiros ficam de fora. Os do FANZA recebem a tag `18+` e seguem a opção **Mostrar jogos +18**. Os jogos ficam em `~/.config/web-avenger/dmm-games.json`, com as mesmas regras do itch.io: editados e removidos ficam no próprio arquivo, e removidos não voltam.

Os jogos do DMM são bloqueados fora do Japão: use uma VPN no sistema. Na primeira vez que abrir um jogo do FANZA, o site pede a verificação de idade. **Desconectar** apaga os cookies do DMM e do FANZA.

## Áudio AAC

As builds do CEF não trazem codecs proprietários, então o Chromium embutido não decodifica AAC — o formato que o Unity WebGL usa para áudio comprimido (jogos ficariam mudos). O launcher intercepta `decodeAudioData` para buffers MP4/M4A (`assets/audio-fallback.js`) e decodifica em Rust com o [Symphonia](https://github.com/pdeljanov/Symphonia) (`src/audio.rs`). MP3, Ogg/Vorbis e Opus continuam com o decodificador do Chromium.

## Dados

Os caminhos acima são os do Linux. No Windows:

| Linux | Windows | Conteúdo |
|-------|---------|----------|
| `~/.config/web-avenger/` | `%APPDATA%\web-avenger\` | `games.json`, `itch-games.json`, `settings.json`, `itch-token.enc` |
| `~/.local/share/web-avenger/` | `%LOCALAPPDATA%\web-avenger\` | perfis do Chromium e `playtime.json` |

Os perfis do Chromium ficam em `~/.local/share/web-avenger/` (`%LOCALAPPDATA%\web-avenger\` no Windows):

- **Perfil principal** (`Default/`): a Biblioteca e os jogos importados do itch.io (que precisam da sessão do itch.io feita no app), com cache, cookies, IndexedDB e localStorage.
- **Um perfil por jogo manual** (`game-<id>/`): cada jogo da Fonte "Manual" roda isolado, como numa janela anônima, mas os dados persistem entre sessões. Um jogo não enxerga cookies nem saves dos outros, nem a sessão do itch.io. Na primeira vez que um jogo manual abre com perfil próprio, o IndexedDB da origem dele é copiado do perfil principal (onde ficam os saves de Unity/Emscripten); o localStorage antigo não é migrado (é um banco único para todas as origens).

Remover um jogo não apaga a pasta do perfil dele.
