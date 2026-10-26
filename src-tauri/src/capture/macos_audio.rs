//! Lleva el audio de cada fuente a un formato fijo (48 kHz, PCM float
//! intercalado) con tiempos continuos antes de entregarlo al codificador.
//!
//! AVAssetWriter falla si el formato de una pista cambia a mitad de la
//! grabación, y eso pasa con audífonos Bluetooth: al activar el micrófono
//! macOS cambia de perfil y la frecuencia salta (p. ej. de 24 a 16 kHz).

use std::ptr::{self, NonNull};

use objc2_core_audio_types::AudioStreamBasicDescription;
use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    kCMBlockBufferAssureMemoryNowFlag, CMAudioFormatDescriptionCreate,
    CMAudioSampleBufferCreateReadyWithPacketDescriptions, CMBlockBuffer, CMFormatDescription,
    CMSampleBuffer as RawSampleBuffer, CMTime,
};
use screencapturekit::prelude::*;

pub const OUTPUT_RATE: u32 = 48_000;

const K_AUDIO_FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
const FLAG_IS_FLOAT: u32 = 1 << 0;
const FLAG_IS_BIG_ENDIAN: u32 = 1 << 1;
const FLAG_IS_SIGNED_INTEGER: u32 = 1 << 2;
const FLAG_IS_PACKED: u32 = 1 << 3;
const FLAG_IS_NON_INTERLEAVED: u32 = 1 << 5;

/// Desfase máximo entre el reloj de la fuente y el nuestro antes de
/// rellenar con silencio (un hueco real, no simple fluctuación).
const MAX_DRIFT_SECONDS: f64 = 0.03;
/// Silencio máximo que se genera de una vez (los huecos largos se cubren por partes).
const MAX_SILENCE_CHUNK_FRAMES: i64 = 2 * OUTPUT_RATE as i64;
/// Huecos más largos que esto no se rellenan con silencio.
const MAX_GAP_FILL_SECONDS: f64 = 10.0;

pub struct AudioConformer {
    channels: usize,
    format: CFRetained<CMFormatDescription>,
    /// Posición del próximo cuadro de salida, en cuadros a 48 kHz.
    next_frame: Option<i64>,
    resampler: LinearResampler,
    /// Último formato de entrada visto, para diagnosticar fallos.
    pub source_format: String,
}

// SAFETY: solo se usa detrás del Mutex del escritor.
unsafe impl Send for AudioConformer {}

impl AudioConformer {
    pub fn new(channels: usize) -> Result<Self, String> {
        let asbd = AudioStreamBasicDescription {
            mSampleRate: f64::from(OUTPUT_RATE),
            mFormatID: K_AUDIO_FORMAT_LINEAR_PCM,
            mFormatFlags: FLAG_IS_FLOAT | FLAG_IS_PACKED,
            mBytesPerPacket: (4 * channels) as u32,
            mFramesPerPacket: 1,
            mBytesPerFrame: (4 * channels) as u32,
            mChannelsPerFrame: channels as u32,
            mBitsPerChannel: 32,
            mReserved: 0,
        };
        let mut out: *const CMFormatDescription = ptr::null();
        let status = unsafe {
            CMAudioFormatDescriptionCreate(
                None,
                NonNull::from(&asbd),
                0,
                ptr::null(),
                0,
                ptr::null(),
                None,
                NonNull::from(&mut out),
            )
        };
        let format = NonNull::new(out.cast_mut())
            .filter(|_| status == 0)
            .map(|p| unsafe { CFRetained::from_raw(p) })
            .ok_or_else(|| {
                tr!(
                    format!("No se pudo preparar el formato de audio ({status})."),
                    format!("Couldn't set up the audio format ({status})."),
                )
            })?;
        Ok(Self {
            channels,
            format,
            next_frame: None,
            resampler: LinearResampler::new(channels),
            source_format: String::new(),
        })
    }

    /// Fija el inicio de la pista (el primer cuadro de video), para que el
    /// audio que llegue después quede alineado con él.
    pub fn start_at(&mut self, seconds: f64) {
        self.next_frame.get_or_insert((seconds * f64::from(OUTPUT_RATE)) as i64);
    }

    /// Silencio hasta `seconds` si la pista va por detrás (no llega audio
    /// porque no suena nada). Devuelve como mucho 2 s por llamada.
    pub fn silence_until(&mut self, seconds: f64) -> Option<CFRetained<RawSampleBuffer>> {
        let next = self.next_frame?;
        let target = (seconds * f64::from(OUTPUT_RATE)) as i64;
        if target <= next {
            return None;
        }
        let frames = (target - next).min(MAX_SILENCE_CHUNK_FRAMES) as usize;
        self.resampler.reset();
        self.make_sample_buffer(&vec![0.0; frames * self.channels])
    }

    /// Convierte un búfer de la fuente. Devuelve `None` si no hay nada que
    /// escribir todavía (o el formato de entrada no se reconoce).
    pub fn convert(&mut self, sample: &CMSampleBuffer) -> Option<CFRetained<RawSampleBuffer>> {
        let desc = sample.format_description()?;
        let rate = desc.audio_sample_rate()?;
        let in_channels = desc.audio_channel_count()? as usize;
        let bits = desc.audio_bits_per_channel()?;
        let flags = desc.audio_format_flags()?;
        self.source_format = format!(
            "{rate} Hz, {in_channels} canal(es), {bits} bits, {}",
            if flags & FLAG_IS_FLOAT != 0 { "float" } else { "entero" }
        );

        let input = read_frames(sample, in_channels, bits, flags, self.channels)?;
        let pts = sample.presentation_timestamp();
        let pts_seconds = pts.value as f64 / f64::from(pts.timescale.max(1));

        let mut output = Vec::new();
        let expected = *self.next_frame.get_or_insert((pts_seconds * f64::from(OUTPUT_RATE)) as i64);
        let drift = pts_seconds - expected as f64 / f64::from(OUTPUT_RATE);
        if drift > MAX_DRIFT_SECONDS && drift < MAX_GAP_FILL_SECONDS {
            // Hueco (p. ej. el Bluetooth cambiando de perfil): se rellena con
            // silencio para que la pista siga alineada con el video.
            output.resize((drift * f64::from(OUTPUT_RATE)) as usize * self.channels, 0.0);
            self.resampler.reset();
        } else if drift >= MAX_GAP_FILL_SECONDS {
            self.next_frame = Some((pts_seconds * f64::from(OUTPUT_RATE)) as i64);
            self.resampler.reset();
        }
        // Un salto hacia atrás se ignora: la salida sigue siendo continua.

        self.resampler.process(&input, rate, f64::from(OUTPUT_RATE), &mut output);
        if output.is_empty() {
            return None;
        }
        self.make_sample_buffer(&output)
    }

    fn make_sample_buffer(&mut self, interleaved: &[f32]) -> Option<CFRetained<RawSampleBuffer>> {
        let frames = interleaved.len() / self.channels;
        let bytes = frames * self.channels * 4;
        let start = self.next_frame?;

        let mut block: *mut CMBlockBuffer = ptr::null_mut();
        let status = unsafe {
            CMBlockBuffer::create_with_memory_block(
                None,
                ptr::null_mut(),
                bytes,
                None,
                ptr::null(),
                0,
                bytes,
                kCMBlockBufferAssureMemoryNowFlag,
                NonNull::from(&mut block),
            )
        };
        let block = NonNull::new(block).filter(|_| status == 0)?;
        let block = unsafe { CFRetained::from_raw(block) };
        let status = unsafe {
            CMBlockBuffer::replace_data_bytes(
                NonNull::new(interleaved.as_ptr() as *mut _)?,
                &block,
                0,
                bytes,
            )
        };
        if status != 0 {
            return None;
        }

        let mut out: *mut RawSampleBuffer = ptr::null_mut();
        let status = unsafe {
            CMAudioSampleBufferCreateReadyWithPacketDescriptions(
                None,
                &block,
                &self.format,
                frames as isize,
                CMTime::new(start, OUTPUT_RATE as i32),
                ptr::null(),
                NonNull::from(&mut out),
            )
        };
        let out = NonNull::new(out).filter(|_| status == 0)?;
        self.next_frame = Some(start + frames as i64);
        Some(unsafe { CFRetained::from_raw(out) })
    }
}

/// Lee los cuadros del búfer como f32 intercalado con `out_channels`
/// canales: mono promedia, estéreo duplica una fuente mono.
fn read_frames(
    sample: &CMSampleBuffer,
    in_channels: usize,
    bits: u32,
    flags: u32,
    out_channels: usize,
) -> Option<Vec<f32>> {
    if in_channels == 0 || flags & FLAG_IS_BIG_ENDIAN != 0 {
        return None;
    }
    let decode: fn(&[u8]) -> f32 = match (flags & FLAG_IS_FLOAT != 0, bits) {
        (true, 32) => |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        (false, 16) if flags & FLAG_IS_SIGNED_INTEGER != 0 => {
            |b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0
        }
        (false, 32) if flags & FLAG_IS_SIGNED_INTEGER != 0 => {
            |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0
        }
        _ => return None,
    };
    let width = (bits / 8) as usize;

    let list = sample.audio_buffer_list().ok()?;
    // channels[c] = muestras del canal c.
    let mut channels: Vec<Vec<f32>> = vec![Vec::new(); in_channels];
    if flags & FLAG_IS_NON_INTERLEAVED != 0 {
        for (c, buffer) in (&list).into_iter().take(in_channels).enumerate() {
            channels[c] = buffer.data().chunks_exact(width).map(decode).collect();
        }
    } else {
        let buffer = (&list).into_iter().next()?;
        for (i, chunk) in buffer.data().chunks_exact(width).enumerate() {
            channels[i % in_channels].push(decode(chunk));
        }
    }

    let frames = channels.iter().map(Vec::len).min().unwrap_or(0);
    let mut out = Vec::with_capacity(frames * out_channels);
    for f in 0..frames {
        if out_channels == 1 {
            out.push(channels.iter().map(|c| c[f]).sum::<f32>() / in_channels as f32);
        } else {
            for c in 0..out_channels {
                out.push(channels[c.min(in_channels - 1)][f]);
            }
        }
    }
    Some(out)
}

/// Remuestreo lineal con estado entre búferes. Basta para voz y para el
/// caso común (48 kHz → 48 kHz), donde no se toca la señal.
struct LinearResampler {
    channels: usize,
    /// Último cuadro del búfer anterior, para interpolar en la unión.
    last: Vec<f32>,
    /// Posición fraccional del próximo cuadro de salida respecto del inicio
    /// del búfer actual (−1 = el cuadro `last`).
    position: f64,
    primed: bool,
}

impl LinearResampler {
    fn new(channels: usize) -> Self {
        Self { channels, last: vec![0.0; channels], position: 0.0, primed: false }
    }

    fn reset(&mut self) {
        self.primed = false;
        self.position = 0.0;
    }

    fn process(&mut self, input: &[f32], in_rate: f64, out_rate: f64, out: &mut Vec<f32>) {
        let ch = self.channels;
        let frames = input.len() / ch;
        if frames == 0 {
            return;
        }
        if (in_rate - out_rate).abs() < 0.5 {
            out.extend_from_slice(&input[..frames * ch]);
            self.last.copy_from_slice(&input[(frames - 1) * ch..frames * ch]);
            self.primed = true;
            self.position = 0.0;
            return;
        }

        let step = in_rate / out_rate;
        if !self.primed {
            self.last.copy_from_slice(&input[..ch]);
            self.position = 0.0;
            self.primed = true;
        }
        // Cuadro i de la entrada; i = −1 es el último del búfer anterior.
        let frame = |i: isize, c: usize| -> f32 {
            if i < 0 { self.last[c] } else { input[i as usize * ch + c] }
        };
        while self.position < (frames - 1) as f64 {
            let i = self.position.floor();
            let t = (self.position - i) as f32;
            let i = i as isize;
            for c in 0..ch {
                let a = frame(i, c);
                let b = frame(i + 1, c);
                out.push(a + (b - a) * t);
            }
            self.position += step;
        }
        self.position -= frames as f64;
        self.last.copy_from_slice(&input[(frames - 1) * ch..frames * ch]);
    }
}

#[cfg(test)]
mod tests {
    use super::LinearResampler;

    #[test]
    fn upsamples_16k_to_48k_continuously_across_buffers() {
        let tone = |n: usize| (n as f32 * 0.05).sin();
        let input: Vec<f32> = (0..1600).map(tone).collect();
        let mut resampler = LinearResampler::new(1);
        let mut out = Vec::new();
        for chunk in input.chunks(160) {
            resampler.process(chunk, 16_000.0, 48_000.0, &mut out);
        }
        // Tres cuadros de salida por cada uno de entrada (±1 por el borde).
        assert!((out.len() as i64 - 4800).abs() <= 3, "{} cuadros", out.len());
        // Cada cuadro de salida k coincide con la señal en k/3, sin saltos en las uniones.
        for (k, v) in out.iter().enumerate() {
            let expected = (k as f32 / 3.0 * 0.05).sin();
            assert!((v - expected).abs() < 1e-3, "cuadro {k}: {v} vs {expected}");
        }
    }

    #[test]
    fn same_rate_passes_through_untouched() {
        let input: Vec<f32> = (0..96).map(|n| n as f32).collect();
        let mut resampler = LinearResampler::new(2);
        let mut out = Vec::new();
        resampler.process(&input, 48_000.0, 48_000.0, &mut out);
        assert_eq!(out, input);
    }
}
