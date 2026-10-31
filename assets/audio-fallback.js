// Áudio AAC (MP4/M4A) no `decodeAudioData`: o CEF não traz esse codec proprietário,
// então os jogos que o usam (ex.: Unity WebGL) ficam mudos. Os buffers MP4 são
// decodificados pelo launcher (`__avengerDecodeAudio`, em Rust).
(function avengerAudioFallback() {
  const decodeNative = window.__avengerDecodeAudio;
  delete window.__avengerDecodeAudio;
  if (typeof decodeNative !== "function" || typeof BaseAudioContext === "undefined") return;
  if (new Audio().canPlayType('audio/mp4; codecs="mp4a.40.2"')) return; // o Chromium já decodifica

  // Contêiner MP4/M4A: `....ftyp` no início.
  function isMp4(buffer) {
    if (!(buffer instanceof ArrayBuffer) || buffer.byteLength < 12) return false;
    const b = new Uint8Array(buffer, 4, 4);
    return b[0] === 0x66 && b[1] === 0x74 && b[2] === 0x79 && b[3] === 0x70;
  }

  function toAudioBuffer(context, bytes) {
    const view = new DataView(bytes);
    const sampleRate = view.getUint32(0, true);
    const channels = view.getUint32(4, true);
    const frames = view.getUint32(8, true);
    const audio = context.createBuffer(channels, Math.max(frames, 1), sampleRate);
    for (let c = 0; c < channels; c++) {
      audio.copyToChannel(new Float32Array(bytes, 12 + c * frames * 4, frames), c);
    }
    return audio;
  }

  const original = BaseAudioContext.prototype.decodeAudioData;
  BaseAudioContext.prototype.decodeAudioData = function decodeAudioData(data, success, error) {
    if (!isMp4(data)) return original.apply(this, arguments);
    const promise = new Promise((resolve, reject) => {
      // Assíncrono como o original; o buffer de entrada é "consumido" como no original.
      setTimeout(() => {
        try {
          resolve(toAudioBuffer(this, decodeNative(data)));
        } catch (err) {
          reject(new DOMException(`Falha ao decodificar AAC: ${err.message ?? err}`, "EncodingError"));
        }
      });
    });
    promise.then((buffer) => success?.(buffer), (err) => error?.(err));
    // Com callbacks, quem chama normalmente ignora a promise: evita "unhandled rejection".
    if (error) promise.catch(() => {});
    return promise;
  };
})();
