// Maximiza players Unity WebGL: o template padrão desenha o jogo num canvas de
// tamanho fixo (ex.: 1280x720) com um rodapé. Aqui o canvas ocupa a janela inteira,
// mantendo a proporção original; o Unity acompanha o tamanho do canvas na tela
// (`matchWebGLToCanvasSize`), então a resolução de renderização sobe junto.
//
// Também maximiza o iframe dos jogos do DMM GAMES / FANZA GAMES (ver `avengerDmmFit`).
(function avengerPlayerFit() {
  // Templates conhecidos: Unity 2020+ e o antigo UnityLoader (2019 e anteriores).
  const TEMPLATES = [
    { container: "#unity-container", player: "#unity-canvas", hide: "#unity-footer" },
    { container: ".webgl-content", player: "#unityContainer, #gameContainer", hide: ".webgl-content .footer" },
  ];

  function fit() {
    const template = TEMPLATES.find((t) => document.querySelector(t.container) && document.querySelector(t.player));
    if (!template) return;
    const player = document.querySelector(template.player);
    const width = parseFloat(player.getAttribute("width") || player.style.width) || player.clientWidth;
    const height = parseFloat(player.getAttribute("height") || player.style.height) || player.clientHeight;
    const aspect = width > 0 && height > 0 ? width / height : 16 / 9;

    const style = document.createElement("style");
    style.id = "web-avenger-player-fit";
    style.textContent = `
      html, body { margin: 0 !important; width: 100% !important; height: 100% !important;
                   overflow: hidden !important; background: #000 !important; }
      ${template.container} {
        position: fixed !important; inset: 0 !important; left: 0 !important; top: 0 !important;
        transform: none !important; margin: 0 !important; width: 100% !important; height: 100% !important;
        display: flex !important; align-items: center !important; justify-content: center !important;
      }
      ${template.player.split(",").map((s) => `${template.container} ${s.trim()}`).join(", ")} {
        width: min(100vw, calc(100vh * ${aspect})) !important;
        height: min(100vh, calc(100vw / ${aspect})) !important;
        flex: none !important;
      }
      ${template.hide} { display: none !important; }
    `;
    document.head.append(style);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", fit, { once: true });
  } else {
    fit();
  }
})();

// DMM GAMES / FANZA GAMES: a página de jogar mostra o jogo num iframe de outro domínio
// (`#game_frame`, do `osapi`), que por sua vez carrega a página do jogo, muitas vezes
// com banners e notícias abaixo do jogo. Nada disso pode ser redimensionado por dentro:
// - nos iframes do DMM, cada frame acha o maior canvas visível (o jogo) e informa a
//   posição dele ao frame pai, somando a posição dos iframes filhos até o `#game_frame`;
// - a página de jogar recorta o `#game_frame` na área do jogo (`clip-path`) e amplia o
//   recorte com `transform`, mantendo a proporção e centralizado. Entre o recorte e o
//   iframe inteiro (ex.: jogo + barra da loja), usa o mais próximo de 16:9. O navegador
//   ajusta os cliques à escala;
// - a escala desce de volta para os frames, que passam a informar `devicePixelRatio`
//   multiplicado por ela: motores que seguem o dpr (Unity, Cocos...) renderizam na
//   resolução final em vez de terem a imagem esticada.
(function avengerDmmFit() {
  const DMM_PLAY = /^https:\/\/play\.games\.dmm\.(com|co\.jp)$/;
  const MESSAGE = "avenger-dmm-game-rect";
  const SCALE_MESSAGE = "avenger-dmm-scale";

  if (window !== window.top) {
    if ([...(location.ancestorOrigins ?? [])].some((origin) => DMM_PLAY.test(origin))) {
      raisePixelRatio();
      reportGameRect();
    }
    return;
  }

  // ---- Iframes do DMM: resolução do canvas acompanha a ampliação ----
  // Roda antes do código da página. O Unity relê o dpr a cada quadro; outros motores só
  // ao iniciar, por isso a última escala fica guardada para a próxima vez que o jogo abrir.
  function raisePixelRatio() {
    const KEY = "avenger-dmm-scale";
    const realRatio = window.devicePixelRatio || 1;
    let scale = 1;
    try { scale = Math.max(1, +localStorage.getItem(KEY) || 1); } catch {}
    try {
      Object.defineProperty(window, "devicePixelRatio", { get: () => realRatio * scale, configurable: true });
    } catch {
      return;
    }
    addEventListener("message", (event) => {
      if (event.source !== parent || event.data?.type !== SCALE_MESSAGE) return;
      for (const frame of document.querySelectorAll("iframe")) frame.contentWindow?.postMessage(event.data, "*");
      const next = Math.max(1, +event.data.scale || 1);
      if (Math.abs(next - scale) < 0.01) return;
      scale = next;
      try { localStorage.setItem(KEY, String(scale)); } catch {}
      dispatchEvent(new Event("resize")); // motores que recalculam o canvas no resize
    });
  }
  if (!DMM_PLAY.test(location.origin)) return;

  // ---- Iframes do DMM: informa ao pai onde está o jogo (em coordenadas deste frame) ----
  function reportGameRect() {
    const fromChildren = new Map(); // janela do iframe filho → retângulo do jogo nela
    addEventListener("message", (event) => {
      if (event.data?.type !== MESSAGE) return;
      const frame = [...document.querySelectorAll("iframe")].find((f) => f.contentWindow === event.source);
      if (!frame) return;
      const box = frame.getBoundingClientRect();
      const { x, y, width, height } = event.data.rect;
      fromChildren.set(event.source, { x: x + box.left + frame.clientLeft, y: y + box.top + frame.clientTop, width, height });
    });

    let last = "";
    setInterval(() => {
      let best = null;
      for (const canvas of document.querySelectorAll("canvas")) {
        const r = canvas.getBoundingClientRect();
        if (r.width >= 300 && r.height >= 200 && (!best || r.width * r.height > best.width * best.height)) best = r;
      }
      for (const r of fromChildren.values()) {
        if (!best || r.width * r.height > best.width * best.height) best = r;
      }
      if (!best) return;
      const rect = { x: best.x + scrollX, y: best.y + scrollY, width: best.width, height: best.height };
      const key = JSON.stringify(rect);
      if (key === last) return;
      last = key;
      parent.postMessage({ type: MESSAGE, rect }, "*");
    }, 1000);
  }

  // ---- Página de jogar ----
  function fit(frame) {
    const style = document.createElement("style");
    style.id = "web-avenger-player-fit";
    style.textContent = `
      html, body { overflow: hidden !important; background: #000 !important; }
      body * { visibility: hidden !important; }
      #game_frame {
        visibility: visible !important; position: fixed !important; left: 0 !important; top: 0 !important;
        margin: 0 !important; z-index: 2147483647 !important; transform-origin: 0 0 !important;
        transform: translate(var(--avenger-x), var(--avenger-y)) scale(var(--avenger-scale)) !important;
        clip-path: var(--avenger-clip, none) !important;
      }
    `;
    document.head.append(style);

    let game = null; // área do jogo dentro do iframe, informada pelos frames do DMM
    addEventListener("message", (event) => {
      if (event.source !== frame.contentWindow || event.data?.type !== MESSAGE) return;
      game = event.data.rect;
      update();
    });

    // `offsetWidth/Height` não mudam com o `transform`: é o tamanho nativo do iframe.
    function update() {
      const frameWidth = frame.offsetWidth, frameHeight = frame.offsetHeight;
      if (!frameWidth || !frameHeight) return;
      // O enquadramento mais próximo de 16:9 (distância em log, para "largo demais" e
      // "alto demais" pesarem igual).
      const whole = { x: 0, y: 0, width: frameWidth, height: frameHeight };
      const distance = (r) => Math.abs(Math.log(r.width / r.height / (16 / 9)));
      const area = game && distance(game) < distance(whole) ? game : whole;
      const scale = Math.min(innerWidth / area.width, innerHeight / area.height);
      frame.style.setProperty("--avenger-scale", scale);
      frame.contentWindow?.postMessage({ type: SCALE_MESSAGE, scale }, "*");
      frame.style.setProperty("--avenger-x", `${(innerWidth - area.width * scale) / 2 - area.x * scale}px`);
      frame.style.setProperty("--avenger-y", `${(innerHeight - area.height * scale) / 2 - area.y * scale}px`);
      frame.style.setProperty("--avenger-clip", area !== whole
        ? `inset(${area.y}px ${frameWidth - area.x - area.width}px ${frameHeight - area.y - area.height}px ${area.x}px)`
        : "none");
    }
    update();
    addEventListener("resize", update);
    // O iframe pode mudar de tamanho depois (a página atualiza as variáveis dele).
    new ResizeObserver(update).observe(frame);
  }

  // A página é uma SPA: o iframe aparece depois do carregamento.
  function find() {
    const frame = document.getElementById("game_frame");
    if (!frame || !document.head) return false;
    fit(frame);
    return true;
  }
  if (find()) return;
  const observer = new MutationObserver(() => { if (find()) observer.disconnect(); });
  observer.observe(document, { childList: true, subtree: true });
})();
