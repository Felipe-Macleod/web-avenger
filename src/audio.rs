//! Decodificação de áudio AAC (MP4/M4A) para o `decodeAudioData` das páginas.
//!
//! As builds do CEF vêm sem codecs proprietários, então o Chromium não decodifica
//! AAC — o formato que o Unity WebGL usa para áudio comprimido. O processo de
//! renderização decodifica aqui (Symphonia, Rust puro) e o script
//! `assets/audio-fallback.js` monta o `AudioBuffer`.

use std::io::Cursor;
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

/// Áudio decodificado, com as amostras separadas por canal (planar).
pub struct Decoded {
    pub sample_rate: u32,
    pub channels: usize,
    pub frames: usize,
    pub samples: Vec<f32>,
}

impl Decoded {
    /// `[u32 sample_rate, u32 channels, u32 frames, f32 amostras...]` em little-endian,
    /// lido pelo script com `DataView`/`Float32Array`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12 + self.samples.len() * 4);
        for value in [self.sample_rate, self.channels as u32, self.frames as u32] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }
}

pub fn decode(data: Vec<u8>) -> Result<Decoded, String> {
    let stream = MediaSourceStream::new(Box::new(Cursor::new(data)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("formato não reconhecido: {e}"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("sem faixa de áudio")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("codec não suportado: {e}"))?;

    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut channels = track.codec_params.channels.map_or(0, |c| c.count());
    let mut interleaved: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(audio) => audio,
            // Um pacote corrompido não deve derrubar o som inteiro.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *audio.spec();
        sample_rate = spec.rate;
        channels = spec.channels.count();
        let mut buffer = SampleBuffer::<f32>::new(audio.capacity() as u64, spec);
        buffer.copy_interleaved_ref(audio);
        interleaved.extend_from_slice(buffer.samples());
    }

    if channels == 0 {
        return Err("áudio sem canais".into());
    }
    let frames = interleaved.len() / channels;
    let mut samples = vec![0.0; frames * channels];
    for (i, sample) in interleaved.iter().take(frames * channels).enumerate() {
        samples[(i % channels) * frames + i / channels] = *sample;
    }
    Ok(Decoded { sample_rate, channels, frames, samples })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seno de 440 Hz, 0,5 s, estéreo, AAC-LC (gerado com ffmpeg).
    const SINE: &[u8] = include_bytes!("../tests/fixtures/sine.m4a");

    #[test]
    fn decodifica_aac_em_mp4() {
        let audio = decode(SINE.to_vec()).expect("AAC deveria decodificar");
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.channels, 2);
        // ~0,5 s (o AAC acrescenta alguns frames de preenchimento).
        assert!((21_000..24_000).contains(&audio.frames), "frames = {}", audio.frames);
        let peak = audio.samples.iter().fold(0f32, |max, s| max.max(s.abs()));
        // O seno do ffmpeg tem amplitude 1/8 por padrão.
        assert!(peak > 0.05, "áudio veio mudo (pico {peak})");
        assert_eq!(audio.to_bytes().len(), 12 + audio.samples.len() * 4);
    }

    #[test]
    fn rejeita_dados_que_nao_sao_audio() {
        assert!(decode(b"isto nao e um mp4".to_vec()).is_err());
    }
}
