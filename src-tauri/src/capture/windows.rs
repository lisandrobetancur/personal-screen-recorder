//! Motor de Windows: Windows Graphics Capture captura pantalla o ventana,
//! WASAPI captura el audio del sistema (loopback) y el micrófono, y Media
//! Foundation codifica por hardware a MP4 (vía `windows-capture`).
//!
//! Diferencias con Mac, por ahora: el codificador admite una sola pista de
//! audio, así que sistema y micrófono se mezclan en ella; y el MP4 se cierra
//! al detener (no se escribe fragmentado).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder, VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use super::{Microphone, RecordOptions, Source, SourceKind};

const SAMPLE_RATE: usize = 48_000;
const CHANNELS: usize = 2;
/// El codificador AAC de Windows admite como máximo 192 kbps.
const AUDIO_BITRATE: u32 = 192_000;
/// Cada cuánto el mezclador entrega audio al codificador.
const MIX_INTERVAL: Duration = Duration::from_millis(10);
/// Audio acumulado máximo por fuente antes de descartar lo más viejo.
const MAX_BACKLOG_FRAMES: usize = SAMPLE_RATE / 2;
/// H.264 no admite más de 4096×2304; por encima se usa HEVC.
const H264_MAX: (u32, u32) = (4096, 2304);

type SharedEncoder = Arc<Mutex<Option<VideoEncoder>>>;

pub fn list_sources() -> Result<Vec<Source>, String> {
    let mut sources = Vec::new();

    let monitors = Monitor::enumerate().map_err(|e| format!("No se pudieron listar las pantallas: {e}"))?;
    for (i, monitor) in monitors.iter().enumerate() {
        let (Ok(width), Ok(height)) = (monitor.width(), monitor.height()) else { continue };
        sources.push(Source {
            kind: SourceKind::Display,
            // Posición en la lista (desde 1), como `Monitor::from_index`.
            id: (i + 1) as u32,
            name: format!("Pantalla {} — {width}×{height}", i + 1),
            width,
            height,
        });
    }

    let own_pid = std::process::id();
    let mut windows: Vec<Source> = Window::enumerate()
        .map_err(|e| format!("No se pudieron listar las ventanas: {e}"))?
        .into_iter()
        .filter(|w| w.is_valid() && w.process_id().ok() != Some(own_pid))
        .filter_map(|w| {
            let title = w.title().ok().filter(|t| !t.trim().is_empty())?;
            let (width, height) = (w.width().ok()?, w.height().ok()?);
            if width < 80 || height < 60 {
                return None;
            }
            let app = w.process_name().unwrap_or_default();
            let app = app.strip_suffix(".exe").unwrap_or(&app).to_string();
            Some(Source {
                kind: SourceKind::Window,
                // Los HWND caben en 32 bits incluso en Windows de 64 bits.
                id: w.as_raw_hwnd() as usize as u32,
                name: format!("{app} — {title}"),
                width: width as u32,
                height: height as u32,
            })
        })
        .collect();
    windows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    sources.extend(windows);

    Ok(sources)
}

pub fn list_microphones() -> Vec<Microphone> {
    let list = || -> Result<Vec<Microphone>, Box<dyn std::error::Error>> {
        let _ = wasapi::initialize_mta();
        let enumerator = DeviceEnumerator::new()?;
        let default_id = enumerator.get_default_device(&Direction::Capture).and_then(|d| d.get_id()).ok();
        let mut mics = Vec::new();
        for device in &enumerator.get_device_collection(&Direction::Capture)? {
            let device = device?;
            let id = device.get_id()?;
            mics.push(Microphone {
                is_default: Some(&id) == default_id.as_ref(),
                name: device.get_friendlyname()?,
                id,
            });
        }
        Ok(mics)
    };
    list().unwrap_or_default()
}

pub struct Recording {
    capture: Option<CaptureControl<CaptureHandler, Box<dyn std::error::Error + Send + Sync>>>,
    encoder: SharedEncoder,
    stop: Arc<AtomicBool>,
    audio_threads: Vec<JoinHandle<()>>,
    path: PathBuf,
}

impl Recording {
    pub fn start(options: &RecordOptions, path: PathBuf) -> Result<Self, String> {
        let fps = options.fps.clamp(1, 120);
        let (width, height) = source_size(options)?;
        let with_audio = options.system_audio || options.microphone;

        let use_hevc = width > H264_MAX.0 || height > H264_MAX.1;
        // ~0,12 bits por píxel, como en Mac: texto nítido sin archivos gigantes.
        let bitrate = (f64::from(width) * f64::from(height) * f64::from(fps) * 0.12).clamp(6e6, 80e6) as u32;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(width, height)
                .sub_type(if use_hevc { VideoSettingsSubType::HEVC } else { VideoSettingsSubType::H264 })
                .bitrate(bitrate)
                .frame_rate(fps),
            AudioSettingsBuilder::new()
                .bitrate(AUDIO_BITRATE)
                .channel_count(CHANNELS as u32)
                .sample_rate(SAMPLE_RATE as u32)
                .bit_per_sample(16)
                .disabled(!with_audio),
            ContainerSettingsBuilder::new(),
            &path,
        )
        .map_err(|e| format!("No se pudo crear el archivo: {e}"))?;
        let encoder: SharedEncoder = Arc::new(Mutex::new(Some(encoder)));

        let stop = Arc::new(AtomicBool::new(false));
        let audio_threads = if with_audio { start_audio(options, &encoder, &stop)? } else { Vec::new() };

        let cursor = if options.show_cursor { CursorCaptureSettings::WithCursor } else { CursorCaptureSettings::WithoutCursor };
        let interval = MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / f64::from(fps)));
        let flags = Arc::clone(&encoder);
        let capture = match options.source_kind {
            SourceKind::Display => {
                let monitor = Monitor::from_index(options.source_id as usize)
                    .map_err(|_| "La pantalla elegida ya no está conectada.")?;
                CaptureHandler::start_free_threaded(Settings::new(
                    monitor,
                    cursor,
                    DrawBorderSettings::WithoutBorder,
                    SecondaryWindowSettings::Default,
                    interval,
                    DirtyRegionSettings::Default,
                    ColorFormat::Bgra8,
                    flags,
                ))
            }
            SourceKind::Window => {
                let window = Window::from_raw_hwnd(options.source_id as usize as *mut std::ffi::c_void);
                if !window.is_valid() {
                    return Err("La ventana elegida ya no existe.".into());
                }
                CaptureHandler::start_free_threaded(Settings::new(
                    window,
                    cursor,
                    DrawBorderSettings::WithoutBorder,
                    SecondaryWindowSettings::Include,
                    interval,
                    DirtyRegionSettings::Default,
                    ColorFormat::Bgra8,
                    flags,
                ))
            }
        };
        let capture = match capture {
            Ok(c) => c,
            Err(e) => {
                stop.store(true, Ordering::Relaxed);
                return Err(format!("No se pudo iniciar la captura: {e}"));
            }
        };

        Ok(Self { capture: Some(capture), encoder, stop, audio_threads, path })
    }

    pub fn stop(mut self) -> Result<PathBuf, String> {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.audio_threads.drain(..) {
            let _ = thread.join();
        }
        if let Some(capture) = self.capture.take() {
            // Si la ventana se cerró, la captura ya terminó sola.
            let _ = capture.stop();
        }
        let encoder = self.encoder.lock().map_err(|_| "El grabador quedó en mal estado.")?.take();
        encoder
            .ok_or("La grabación ya estaba cerrada.")?
            .finish()
            .map_err(|e| format!("La grabación falló al cerrar el archivo: {e}"))?;
        Ok(std::mem::take(&mut self.path))
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn source_size(options: &RecordOptions) -> Result<(u32, u32), String> {
    let even = |v: u32| v.max(2) & !1;
    match options.source_kind {
        SourceKind::Display => {
            let m = Monitor::from_index(options.source_id as usize)
                .map_err(|_| "La pantalla elegida ya no está conectada.")?;
            Ok((even(m.width().map_err(|e| e.to_string())?), even(m.height().map_err(|e| e.to_string())?)))
        }
        SourceKind::Window => {
            let w = Window::from_raw_hwnd(options.source_id as usize as *mut std::ffi::c_void);
            let width = w.width().map_err(|_| "La ventana elegida ya no existe.")?;
            let height = w.height().map_err(|_| "La ventana elegida ya no existe.")?;
            Ok((even(width.max(2) as u32), even(height.max(2) as u32)))
        }
    }
}

struct CaptureHandler {
    encoder: SharedEncoder,
}

impl GraphicsCaptureApiHandler for CaptureHandler {
    type Flags = SharedEncoder;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { encoder: ctx.flags })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
        if let Some(encoder) = self.encoder.lock().map_err(|_| "codificador bloqueado")?.as_mut() {
            encoder.send_frame(frame)?;
        }
        Ok(())
    }
}

/// Audio de una fuente esperando a mezclarse: f32 intercalado estéreo a 48 kHz.
type Pending = Arc<Mutex<VecDeque<f32>>>;

/// Arranca un hilo por fuente de audio y un mezclador que las suma y las
/// entrega al codificador al ritmo del reloj: si una fuente no manda nada
/// (no suena nada), se rellena con silencio y la pista sigue alineada.
fn start_audio(options: &RecordOptions, encoder: &SharedEncoder, stop: &Arc<AtomicBool>) -> Result<Vec<JoinHandle<()>>, String> {
    let mut sources: Vec<Pending> = Vec::new();
    let mut threads = Vec::new();

    if options.system_audio {
        let pending = Pending::default();
        threads.push(spawn_capture(Endpoint::SystemLoopback, Arc::clone(&pending), Arc::clone(stop))?);
        sources.push(pending);
    }
    if options.microphone {
        let pending = Pending::default();
        let endpoint = Endpoint::Microphone(options.microphone_id.clone());
        threads.push(spawn_capture(endpoint, Arc::clone(&pending), Arc::clone(stop))?);
        sources.push(pending);
    }

    let encoder = Arc::clone(encoder);
    let stop = Arc::clone(stop);
    threads.push(std::thread::spawn(move || mix_loop(&sources, &encoder, &stop)));
    Ok(threads)
}

fn mix_loop(sources: &[Pending], encoder: &SharedEncoder, stop: &AtomicBool) {
    let start = Instant::now();
    let mut frames_sent: usize = 0;
    let mut mixed: Vec<f32> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(MIX_INTERVAL);
        let due = (start.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as usize;
        let frames = due.saturating_sub(frames_sent);
        if frames == 0 {
            continue;
        }

        mixed.clear();
        mixed.resize(frames * CHANNELS, 0.0);
        for source in sources {
            let Ok(mut pending) = source.lock() else { continue };
            // Si una fuente se adelantó al reloj, se descarta lo más viejo.
            let excess = pending.len().saturating_sub((frames + MAX_BACKLOG_FRAMES) * CHANNELS);
            pending.drain(..excess);
            let available = pending.len().min(frames * CHANNELS);
            for (out, sample) in mixed.iter_mut().zip(pending.drain(..available)) {
                *out += sample;
            }
        }

        bytes.clear();
        bytes.extend(mixed.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes()));
        if let Ok(mut guard) = encoder.lock() {
            if let Some(encoder) = guard.as_mut() {
                let _ = encoder.send_audio_buffer(&bytes, 0);
            }
        }
        frames_sent += frames;
    }
}

enum Endpoint {
    SystemLoopback,
    /// `None` = micrófono predeterminado.
    Microphone(Option<String>),
}

/// Abre la fuente en este hilo (para informar errores al empezar) y luego la
/// lee en un hilo propio hasta que `stop` se active.
fn spawn_capture(endpoint: Endpoint, pending: Pending, stop: Arc<AtomicBool>) -> Result<JoinHandle<()>, String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let label = match endpoint {
        Endpoint::SystemLoopback => "el audio del sistema",
        Endpoint::Microphone(_) => "el micrófono",
    };
    let thread = std::thread::spawn(move || {
        if let Err(e) = capture_loop(&endpoint, &pending, &stop, &ready_tx) {
            let _ = ready_tx.send(Err(e.to_string()));
        }
    });
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(thread),
        Ok(Err(e)) => Err(format!("No se pudo capturar {label}: {e}")),
        Err(_) => Err(format!("No se pudo capturar {label}: no respondió.")),
    }
}

fn capture_loop(
    endpoint: &Endpoint,
    pending: &Pending,
    stop: &AtomicBool,
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = wasapi::initialize_mta();
    let enumerator = DeviceEnumerator::new()?;
    let device = match endpoint {
        // Dispositivo de salida + dirección de captura = loopback de lo que suena.
        Endpoint::SystemLoopback => enumerator.get_default_device(&Direction::Render)?,
        Endpoint::Microphone(Some(id)) => enumerator.get_device(id)?,
        Endpoint::Microphone(None) => enumerator.get_default_device(&Direction::Capture)?,
    };
    let mut client = device.get_iaudioclient()?;
    // Windows convierte a este formato, sea cual sea el del dispositivo.
    let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE, CHANNELS, None);
    let (default_period, _) = client.get_device_period()?;
    // Por sondeo y no por eventos: en loopback no hay eventos mientras no suena nada.
    let mode = StreamMode::PollingShared { autoconvert: true, buffer_duration_hns: default_period * 4 };
    client.initialize_client(&format, &Direction::Capture, &mode)?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;
    let _ = ready.send(Ok(()));

    let mut raw: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(5));
        while capture.get_next_packet_size()?.unwrap_or(0) > 0 {
            capture.read_from_device_to_deque(&mut raw)?;
        }
        let whole = raw.len() / 4 * 4;
        if whole == 0 {
            continue;
        }
        let samples: Vec<f32> = raw
            .drain(..whole)
            .collect::<Vec<u8>>()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        if let Ok(mut p) = pending.lock() {
            p.extend(samples);
        }
    }
    client.stop_stream()?;
    Ok(())
}
