// Tradutor de input do Web Avenger: apresenta controles não-Xbox aos jogos como
// um controle Xbox ("standard" mapping + id XInput). Injetado pelo processo de
// renderização antes dos scripts da página; a configuração é passada como argumento.
(function avengerInputTranslation(config) {
  if (!navigator.getGamepads || navigator.getGamepads.avenger) return;

  const chromiumGetGamepads = navigator.getGamepads.bind(navigator);
  const XBOX_ID = "Xbox 360 Controller (XInput STANDARD GAMEPAD)";

  // Função nativa do launcher com os controles que o Chromium não enxerga
  // (Nintendo e PlayStation com os drivers do kernel), lidos direto do joydev.
  const readNativePads = window.__avengerNativePads;
  delete window.__avengerNativePads;
  const nativeState = new Map(); // device -> { seq, timestamp, slot }

  // Ordem do "standard" mapping da Gamepad API (= controle de Xbox).
  const STANDARD_BUTTONS = [
    "A", "B", "X", "Y", "LB", "RB", "LT", "RT", "Back", "Start",
    "LS", "RS", "Up", "Down", "Left", "Right", "Guide",
  ];

  const button = (pressed, value = pressed ? 1 : 0) => ({ pressed, touched: pressed, value });
  const copyButton = (b) => (b ? button(b.pressed, b.value) : button(false));

  function vendorProduct(id) {
    const match = /Vendor: ([0-9a-f]{4}) Product: ([0-9a-f]{4})/i.exec(id);
    return match ? `${match[1]}:${match[2]}`.toLowerCase() : null;
  }

  // Chave do controle nos mapeamentos manuais.
  const vendorKey = (id) => vendorProduct(id) ?? id;

  // Gamepads do Chromium + controles nativos, que ocupam os slots vazios.
  function rawGetGamepads() {
    const pads = Array.from(chromiumGetGamepads());
    if (typeof readNativePads !== "function") return pads;

    let native;
    try {
      native = JSON.parse(readNativePads());
    } catch {
      return pads;
    }
    // Se o Chromium também entrega um controle lido nativamente, fica só o nativo.
    const nativeKeys = new Set(native.map((pad) => vendorProduct(pad.id)));
    pads.forEach((pad, i) => {
      if (pad && nativeKeys.has(vendorProduct(pad.id))) pads[i] = null;
    });

    for (const pad of native) {
      let state = nativeState.get(pad.device);
      if (!state) nativeState.set(pad.device, (state = { seq: -1, timestamp: 0, slot: -1 }));
      if (state.seq !== pad.seq) {
        state.seq = pad.seq;
        state.timestamp = performance.now();
      }
      if (pads[state.slot] !== null) {
        state.slot = pads.indexOf(null);
        if (state.slot < 0) state.slot = pads.push(null) - 1;
      }
      pads[state.slot] = {
        id: pad.id,
        index: state.slot,
        connected: pad.connected,
        timestamp: state.timestamp,
        mapping: "",
        axes: pad.axes,
        buttons: pad.buttons.map((value) => button(value > 0.5, value)),
        buttonCodes: pad.buttonCodes,
        axisCodes: pad.axisCodes,
        vibrationActuator: null,
      };
    }
    return pads;
  }

  const isXbox = (id) => /xbox|xinput/i.test(id) || vendorProduct(id)?.startsWith("045e:");
  const isNintendo = (id) => vendorProduct(id)?.startsWith("057e:") || /nintendo|pro controller|joy-con/i.test(id);

  // Códigos evdev (linux/input-event-codes.h) na ordem standard. Os drivers do kernel
  // (hid-nintendo, hid-playstation, xpad...) usam a posição física: BTN_SOUTH é o
  // botão de baixo, seja ele o A do Xbox, o B da Nintendo ou o ✕ do PlayStation.
  const BTN = {
    SOUTH: 0x130, EAST: 0x131, NORTH: 0x133, WEST: 0x134, TL: 0x136, TR: 0x137,
    TL2: 0x138, TR2: 0x139, SELECT: 0x13a, START: 0x13b, MODE: 0x13c,
    THUMBL: 0x13d, THUMBR: 0x13e,
    DPAD_UP: 0x220, DPAD_DOWN: 0x221, DPAD_LEFT: 0x222, DPAD_RIGHT: 0x223,
  };
  const ABS = { X: 0x00, Y: 0x01, Z: 0x02, RX: 0x03, RY: 0x04, RZ: 0x05, HAT0X: 0x10, HAT0Y: 0x11 };

  // Controles lidos nativamente: converte pelo código evdev de cada botão/eixo.
  function fromEvdev(pad) {
    const btn = (code) => copyButton(pad.buttons[pad.buttonCodes.indexOf(code)]);
    const axis = (code) => {
      const i = pad.axisCodes.indexOf(code);
      return i < 0 ? null : pad.axes[i] ?? 0;
    };
    // Gatilho analógico (repouso em −1) ou, se não houver eixo, o botão digital.
    const trigger = (absCode, btnCode) => {
      const value = axis(absCode);
      if (value === null) return btn(btnCode);
      const amount = Math.min(Math.max((value + 1) / 2, 0), 1);
      return button(amount > 0.1, amount);
    };
    // D-pad em botões ou como hat.
    const dpad = (btnCode, absCode, sign) => {
      if (pad.buttonCodes.includes(btnCode)) return btn(btnCode);
      return button((axis(absCode) ?? 0) * sign > 0.5);
    };
    return {
      buttons: [
        btn(BTN.SOUTH), btn(BTN.EAST), btn(BTN.WEST), btn(BTN.NORTH),
        btn(BTN.TL), btn(BTN.TR), trigger(ABS.Z, BTN.TL2), trigger(ABS.RZ, BTN.TR2),
        btn(BTN.SELECT), btn(BTN.START), btn(BTN.THUMBL), btn(BTN.THUMBR),
        dpad(BTN.DPAD_UP, ABS.HAT0Y, -1), dpad(BTN.DPAD_DOWN, ABS.HAT0Y, 1),
        dpad(BTN.DPAD_LEFT, ABS.HAT0X, -1), dpad(BTN.DPAD_RIGHT, ABS.HAT0X, 1),
        btn(BTN.MODE),
      ],
      axes: [ABS.X, ABS.Y, ABS.RX, ABS.RY].map((code) => axis(code) ?? 0),
    };
  }

  // Mapeamento manual feito na Biblioteca. Cada fonte é `{button}`, `{axis, dir, rest}`
  // (botão vindo de um eixo, ex.: gatilho ou hat) ou `{axis, invert}` (stick); `null` = sem.
  function fromMapping(pad, mapping) {
    const buttons = STANDARD_BUTTONS.map((_, i) => {
      const source = mapping.buttons?.[i];
      if (!source) return button(false);
      if (source.button !== undefined) return copyButton(pad.buttons[source.button]);
      const value = pad.axes[source.axis] ?? source.rest;
      const amount = Math.min(Math.max((value - source.rest) / (source.dir - source.rest), 0), 1);
      return button(amount > 0.5, amount);
    });
    const axes = [0, 1, 2, 3].map((i) => {
      const source = mapping.axes?.[i];
      if (!source) return 0;
      const value = pad.axes[source.axis] ?? 0;
      return source.invert ? -value : value;
    });
    return { buttons, axes };
  }

  function translate(pad) {
    if (!pad || !config.enabled) return pad;
    const mapping = config.mappings?.[vendorKey(pad.id)];
    if (!mapping && isXbox(pad.id)) return pad;

    let buttons;
    let axes;
    if (mapping) {
      ({ buttons, axes } = fromMapping(pad, mapping));
    } else if (pad.mapping === "standard") {
      buttons = Array.from(pad.buttons, copyButton);
      axes = Array.from(pad.axes).slice(0, 4);
    } else if (pad.buttonCodes?.length) {
      ({ buttons, axes } = fromEvdev(pad));
    } else {
      return pad;
    }

    // O standard mapping é por posição; "label" troca A↔B e X↔Y nos controles Nintendo.
    // Um mapeamento manual já reflete a escolha do usuário.
    if (!mapping && config.layout === "label" && isNintendo(pad.id)) {
      [buttons[0], buttons[1]] = [buttons[1], buttons[0]];
      [buttons[2], buttons[3]] = [buttons[3], buttons[2]];
    }

    // Os jogos (wasm-bindgen) checam `instanceof Gamepad`.
    const gamepad = Object.create(Gamepad.prototype);
    const props = {
      id: XBOX_ID,
      index: pad.index,
      connected: pad.connected,
      timestamp: pad.timestamp,
      mapping: "standard",
      axes: Object.freeze(axes),
      buttons: Object.freeze(buttons),
      vibrationActuator: pad.vibrationActuator,
    };
    for (const [key, value] of Object.entries(props)) {
      Object.defineProperty(gamepad, key, { value, enumerable: true });
    }
    return gamepad;
  }

  const getGamepads = function getGamepads() {
    return Array.from(rawGetGamepads(), translate);
  };
  getGamepads.avenger = true;
  Object.defineProperty(navigator, "getGamepads", {
    value: getGamepads,
    configurable: true,
    writable: true,
  });

  // Os eventos também carregam o Gamepad: entrega a versão traduzida.
  for (const type of ["gamepadconnected", "gamepaddisconnected"]) {
    window.addEventListener(type, (event) => {
      if (!config.enabled || !event.gamepad) return;
      Object.defineProperty(event, "gamepad", { value: translate(event.gamepad) });
    }, true);
  }

  if (config.expose) {
    // Só na Biblioteca: o testador de controle lê o input cru e altera a config ao vivo.
    Object.defineProperty(window, "avengerInput", {
      value: { config, rawGetGamepads, translate, vendorKey, STANDARD_BUTTONS },
    });
  }
})
