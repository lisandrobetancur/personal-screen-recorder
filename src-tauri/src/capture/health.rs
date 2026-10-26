//! Señales de que una grabación podría estar quedando vacía: silencio
//! digital absoluto (no llega audio) y pantalla completamente negra. La
//! interfaz las consulta y decide cuándo preguntar.

use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;

/// Por debajo de esto (≈ −100 dBFS) es silencio digital: no llega señal.
/// Un micrófono encendido siempre capta algo de ruido por encima.
const DIGITAL_SILENCE: f32 = 1e-5;

/// La imagen solo se revisa al empezar (los primeros 60 s, con margen).
pub const BLACK_CHECK_WINDOW_SECS: f64 = 65.0;

/// Fracción de la imagen en negro puro a partir de la cual se avisa. No hace
/// falta que sea toda: cuando una plataforma bloquea la grabación suele salir
/// negro solo el recuadro del video, y alrededor se ve el navegador.
const BLACK_FRACTION: f32 = 0.7;

#[derive(Debug, Clone, Copy)]
pub enum AudioSource {
    System,
    Microphone,
}

#[derive(Default)]
pub struct Health {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    started: Option<Instant>,
    system: Track,
    microphone: Track,
    /// Desde cuándo la imagen está negra (None = se ve algo, o aún no se sabe).
    black_since: Option<Instant>,
}

#[derive(Default)]
struct Track {
    last_sound: Option<Instant>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub elapsed_secs: f64,
    pub system_heard: bool,
    pub system_silent_secs: f64,
    pub microphone_heard: bool,
    pub microphone_silent_secs: f64,
    pub black_secs: f64,
}

impl Health {
    pub fn start(&self) {
        if let Ok(mut s) = self.state.lock() {
            *s = State { started: Some(Instant::now()), ..State::default() };
        }
    }

    /// Segundos desde el inicio, para decidir si todavía vale la pena revisar la imagen.
    pub fn elapsed_secs(&self) -> f64 {
        self.state
            .lock()
            .ok()
            .and_then(|s| s.started)
            .map_or(0.0, |t| t.elapsed().as_secs_f64())
    }

    /// Registra un trozo de audio de una fuente (muestras de la propia fuente,
    /// no el silencio de relleno que agrega la app).
    pub fn audio(&self, source: AudioSource, samples: &[f32]) {
        if has_sound(samples) {
            self.heard(source);
        }
    }

    /// Registra que llegó sonido de una fuente.
    pub fn heard(&self, source: AudioSource) {
        if let Ok(mut s) = self.state.lock() {
            let track = match source {
                AudioSource::System => &mut s.system,
                AudioSource::Microphone => &mut s.microphone,
            };
            track.last_sound = Some(Instant::now());
        }
    }

    /// Registra si el último cuadro revisado estaba completamente negro.
    pub fn frame(&self, black: bool) {
        if let Ok(mut s) = self.state.lock() {
            s.black_since = match (black, s.black_since) {
                (true, None) => Some(Instant::now()),
                (true, since) => since,
                (false, _) => None,
            };
        }
    }

    pub fn report(&self) -> HealthReport {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let started = s.started.unwrap_or(now);
        let silent = |t: &Track| now.duration_since(t.last_sound.unwrap_or(started)).as_secs_f64();
        HealthReport {
            elapsed_secs: now.duration_since(started).as_secs_f64(),
            system_heard: s.system.last_sound.is_some(),
            system_silent_secs: silent(&s.system),
            microphone_heard: s.microphone.last_sound.is_some(),
            microphone_silent_secs: silent(&s.microphone),
            black_secs: s.black_since.map_or(0.0, |t| now.duration_since(t).as_secs_f64()),
        }
    }
}

pub fn has_sound(samples: &[f32]) -> bool {
    samples.iter().any(|s| s.abs() > DIGITAL_SILENCE)
}

/// ¿La imagen está mayormente en negro puro? Se muestrea en una cuadrícula de
/// 128 × 72 puntos. `max_black` es el valor más claro que todavía cuenta como
/// negro (los temas oscuros usan grises, no negro puro, y no cuentan).
pub fn is_black(data: &[u8], width: usize, height: usize, row_bytes: usize, pixel_bytes: usize, max_black: u8) -> bool {
    black_fraction(data, width, height, row_bytes, pixel_bytes, max_black) >= BLACK_FRACTION
}

fn black_fraction(data: &[u8], width: usize, height: usize, row_bytes: usize, pixel_bytes: usize, max_black: u8) -> f32 {
    if width == 0 || height == 0 {
        return 0.0;
    }
    let (step_x, step_y) = ((width / 128).max(1), (height / 72).max(1));
    let (mut black, mut total) = (0u32, 0u32);
    for y in (0..height).step_by(step_y) {
        for x in (0..width).step_by(step_x) {
            let i = y * row_bytes + x * pixel_bytes;
            let Some(px) = data.get(i..i + pixel_bytes) else { continue };
            // En BGRA se ignora el alfa (último byte).
            let channels = if pixel_bytes == 4 { &px[..3] } else { px };
            total += 1;
            if channels.iter().all(|&c| c <= max_black) {
                black += 1;
            }
        }
    }
    if total == 0 { 0.0 } else { black as f32 / total as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digital_silence_is_not_sound_but_room_noise_is() {
        assert!(!has_sound(&[0.0; 480]));
        assert!(!has_sound(&[1e-7; 480]));
        assert!(has_sound(&[0.0, 0.0, 3e-4, 0.0]));
    }

    #[test]
    fn detects_black_frames() {
        // Luminancia en rango de video: el negro es 16.
        let (w, h) = (1280, 720);
        let black = vec![16u8; w * h];
        assert!(is_black(&black, w, h, w, 1, 24));

        // Recuadro de video negro que ocupa el 80 % con el navegador alrededor: avisa.
        let mut player = vec![200u8; w * h];
        for y in (h / 10)..(h * 9 / 10) {
            for x in 0..w {
                player[y * w + x] = 16;
            }
        }
        assert!(is_black(&player, w, h, w, 1, 24));

        // Ventana normal con un video negro chico (30 %): no avisa.
        let mut small = vec![200u8; w * h];
        for y in 0..(h * 3 / 10) {
            for x in 0..w {
                small[y * w + x] = 16;
            }
        }
        assert!(!is_black(&small, w, h, w, 1, 24));

        // Tema oscuro (gris #1e1e1e ≈ 42 en luminancia de video): no es negro.
        assert!(!is_black(&vec![42u8; w * h], w, h, w, 1, 24));

        // BGRA: alfa en 255 no cuenta como claro.
        let bgra: Vec<u8> = [0, 0, 0, 255].repeat(128 * 72);
        assert!(is_black(&bgra, 128, 72, 128 * 4, 4, 8));
    }

    #[test]
    fn report_tracks_silence_and_black() {
        let h = Health::default();
        h.start();
        let r = h.report();
        assert!(!r.system_heard && !r.microphone_heard && r.black_secs == 0.0);
        h.audio(AudioSource::Microphone, &[0.01; 10]);
        h.frame(true);
        let r = h.report();
        assert!(r.microphone_heard && !r.system_heard);
        assert!(r.microphone_silent_secs < 1.0);
        h.frame(false);
        assert_eq!(h.report().black_secs, 0.0);
    }
}
