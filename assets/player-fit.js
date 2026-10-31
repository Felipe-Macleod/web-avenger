// Maximiza players Unity WebGL: o template padrão desenha o jogo num canvas de
// tamanho fixo (ex.: 1280x720) com um rodapé. Aqui o canvas ocupa a janela inteira,
// mantendo a proporção original; o Unity acompanha o tamanho do canvas na tela
// (`matchWebGLToCanvasSize`), então a resolução de renderização sobe junto.
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
