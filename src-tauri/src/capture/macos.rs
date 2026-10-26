//! Motor de macOS: ScreenCaptureKit captura pantalla, audio del sistema y
//! micrófono; AVAssetWriter codifica por hardware (VideoToolbox) a MP4.
//!
//! Se graba a un .mov fragmentado cada pocos segundos: si la app se cierra de
//! golpe, el archivo sigue siendo reproducible hasta el último fragmento. Al
//! detener, se copia a .mp4 sin recomprimir. (Escribir el MP4 fragmentado
//! directamente fallaba al cerrarlo ~1 de cada 17 veces, error -16341.)

use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2::runtime::Bool;
use objc2_av_foundation::{
    AVAssetWriter, AVAuthorizationStatus, AVCaptureDevice, AVAssetWriterInput, AVAssetWriterStatus, AVAssetExportPresetPassthrough, AVAssetExportSession, AVAssetExportSessionStatus, AVFileTypeMPEG4, AVFileTypeQuickTimeMovie, AVURLAsset, AVMediaTypeAudio,
    AVMediaTypeVideo, AVVideoAverageBitRateKey, AVVideoCodecKey, AVVideoCodecTypeH264,
    AVVideoCodecTypeHEVC, AVVideoCompressionPropertiesKey, AVVideoExpectedSourceFrameRateKey,
    AVVideoHeightKey, AVVideoMaxKeyFrameIntervalKey, AVVideoProfileLevelH264HighAutoLevel,
    AVVideoProfileLevelKey, AVVideoWidthKey,
};
use objc2_core_media::{CMSampleBuffer as RawSampleBuffer, CMTime};
use objc2_foundation::{
    NSDictionary, NSError, NSNumber, NSOperatingSystemVersion, NSProcessInfo, NSString, NSUnderlyingErrorKey,
    NSURL,
};
use screencapturekit::cm::SCFrameStatus;
use screencapturekit::prelude::*;

use super::macos_audio::{AudioConformer, OUTPUT_RATE};
use super::{Microphone, RecordOptions, Source, SourceKind};

const SAMPLE_RATE: u32 = OUTPUT_RATE;
const SYSTEM_AUDIO_BITRATE: i32 = 320_000;
const MICROPHONE_BITRATE: i32 = 192_000;
/// Cada cuánto se cierra un fragmento del MP4 (lo máximo que se pierde ante un corte).
const FRAGMENT_SECONDS: f64 = 2.0;
/// H.264 no admite más de 4096×2304; por encima se usa HEVC.
const H264_MAX: (u32, u32) = (4096, 2304);

const PERMISSION_HINT: &str = "Activa el permiso en Ajustes del Sistema → Privacidad y seguridad → \
     Grabación de pantalla y audio del sistema, y vuelve a abrir la app.";

pub fn list_sources() -> Result<Vec<Source>, String> {
    let content = shareable_content()?;
    let mut sources = Vec::new();

    for (i, display) in content.displays().iter().enumerate() {
        let (width, height) = display_pixels(display)?;
        sources.push(Source {
            kind: SourceKind::Display,
            id: display.display_id(),
            name: format!("Pantalla {} — {width}×{height}", i + 1),
            width,
            height,
        });
    }

    let own_pid = std::process::id() as i32;
    let mut windows: Vec<Source> = content
        .windows()
        .iter()
        .filter(|w| w.is_on_screen() && w.window_layer() == 0)
        .filter(|w| w.frame().size.width >= 80.0 && w.frame().size.height >= 60.0)
        .filter_map(|w| {
            let app = w.owning_application()?;
            if app.process_id() == own_pid {
                return None;
            }
            let title = w.title().filter(|t| !t.trim().is_empty())?;
            let (width, height) = window_pixels(w).ok()?;
            Some(Source {
                kind: SourceKind::Window,
                id: w.window_id(),
                name: format!("{} — {title}", app.application_name()),
                width,
                height,
            })
        })
        .collect();
    windows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    sources.extend(windows);

    Ok(sources)
}

pub fn list_microphones() -> Vec<Microphone> {
    AudioInputDevice::list()
        .into_iter()
        .map(|d| Microphone { id: d.id, name: d.name, is_default: d.is_default })
        .collect()
}

pub struct Recording {
    stream: SCStream,
    writer: Arc<Mutex<Writer>>,
    path: PathBuf,
}

impl Recording {
    pub fn start(options: &RecordOptions, path: PathBuf) -> Result<Self, String> {
        if options.microphone && !supports_microphone() {
            return Err("Grabar el micrófono requiere macOS 15 o superior.".into());
        }
        if options.microphone {
            ensure_microphone_access()?;
        }

        let content = shareable_content()?;
        let (filter, width, height) = build_filter(&content, options)?;
        let fps = options.fps.clamp(1, 120);

        let mut config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(height)
            .with_fps(fps)
            .with_pixel_format(PixelFormat::YCbCr_420v)
            .with_queue_depth(8)
            .with_shows_cursor(options.show_cursor)
            .with_captures_audio(options.system_audio)
            .with_excludes_current_process_audio(true)
            .with_sample_rate(SAMPLE_RATE as i32)
            .with_channel_count(2)
            .with_shows_mouse_clicks(options.show_clicks)
            .map_err(sc_error)?;
        if options.microphone {
            config = config.with_captures_microphone(true).map_err(sc_error)?;
            if let Some(id) = &options.microphone_id {
                config = config.with_microphone_capture_device_id(id).map_err(sc_error)?;
            }
        }

        let writer = Writer::new(&path, width, height, fps, options.system_audio, options.microphone)?;
        let writer = Arc::new(Mutex::new(writer));

        let mut stream = SCStream::new(&filter, &config).map_err(sc_error)?;
        let mut kinds = vec![SCStreamOutputType::Screen];
        if options.system_audio {
            kinds.push(SCStreamOutputType::Audio);
        }
        if options.microphone {
            kinds.push(SCStreamOutputType::Microphone);
        }
        for kind in kinds {
            let writer = Arc::clone(&writer);
            stream
                .add_output_handler(
                    move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
                        if let Ok(mut w) = writer.lock() {
                            w.append(&sample, kind);
                        }
                    },
                    kind,
                )
                .map_err(sc_error)?;
        }

        if let Err(e) = stream.start_capture() {
            if let Ok(mut w) = writer.lock() {
                w.cancel();
            }
            return Err(format!("No se pudo iniciar la captura: {e}. {PERMISSION_HINT}"));
        }

        Ok(Self { stream, writer, path })
    }

    pub fn stop(self) -> Result<PathBuf, String> {
        // Si la fuente desapareció (ventana cerrada), la captura ya se detuvo sola.
        let _ = self.stream.stop_capture();
        let mut writer = self.writer.lock().map_err(|_| "El grabador quedó en mal estado.")?;
        writer.finish()?;
        // Si la copia a MP4 falla, se conserva el .mov: la grabación no se pierde.
        Ok(convert_to_mp4(&self.path).unwrap_or(self.path))
    }
}

/// Copia el .mov a un .mp4 al lado (mismas pistas, sin recomprimir) y
/// borra el .mov.
fn convert_to_mp4(mov: &std::path::Path) -> Result<PathBuf, String> {
    let mp4 = mov.with_extension("mp4");
    let source = NSURL::from_file_path(mov).ok_or("Ruta inválida.")?;
    let target = NSURL::from_file_path(&mp4).ok_or("Ruta inválida.")?;
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&source, None) };
    let session = unsafe {
        AVAssetExportSession::exportSessionWithAsset_presetName(&asset, AVAssetExportPresetPassthrough)
    }
    .ok_or("No se pudo preparar la conversión a MP4.")?;
    unsafe {
        session.setOutputURL(Some(&target));
        session.setOutputFileType(Some(constant(AVFileTypeMPEG4)));
        session.setShouldOptimizeForNetworkUse(true);
    }

    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move || {
        let _ = tx.send(());
    });
    unsafe { session.exportAsynchronouslyWithCompletionHandler(&handler) };
    // Solo copia datos: incluso horas de grabación tardan segundos.
    rx.recv_timeout(Duration::from_secs(600)).map_err(|_| "La conversión a MP4 tardó demasiado.")?;

    if unsafe { session.status() } == AVAssetExportSessionStatus::Completed {
        let _ = std::fs::remove_file(mov);
        Ok(mp4)
    } else {
        let _ = std::fs::remove_file(&mp4);
        Err(unsafe { session.error() }.map(|e| describe_error(&e)).unwrap_or_default())
    }
}

struct Writer {
    writer: Retained<AVAssetWriter>,
    video: Retained<AVAssetWriterInput>,
    system_audio: Option<AudioTrack>,
    microphone: Option<AudioTrack>,
    session_started: bool,
    /// Fin del último cuadro de video, donde se cierra la grabación.
    session_end: Option<CMTime>,
    done: bool,
    /// Primer búfer que el codificador rechazó, con contexto para diagnosticar.
    first_failure: Option<String>,
}

struct AudioTrack {
    input: Retained<AVAssetWriterInput>,
    conformer: AudioConformer,
    label: &'static str,
}

impl AudioTrack {
    fn new(channels: usize, bitrate: i32, label: &'static str) -> Result<Self, String> {
        Ok(Self {
            input: new_input(constant(unsafe { AVMediaTypeAudio }), &audio_settings(channels as u32, bitrate)),
            conformer: AudioConformer::new(channels)?,
            label,
        })
    }

    /// `false` solo si el codificador rechazó el búfer; si no estaba listo,
    /// el búfer se descarta (en tiempo real no se puede esperar).
    fn append(&self, buffer: &RawSampleBuffer) -> bool {
        unsafe { !self.input.isReadyForMoreMediaData() || self.input.appendSampleBuffer(buffer) }
    }
}

// SAFETY: el escritor y sus entradas solo se usan detrás del Mutex de
// `Recording`, nunca desde dos hilos a la vez.
unsafe impl Send for Writer {}

impl Writer {
    fn new(
        path: &std::path::Path,
        width: u32,
        height: u32,
        fps: u32,
        system_audio: bool,
        microphone: bool,
    ) -> Result<Self, String> {
        let url = NSURL::from_file_path(path).ok_or("Ruta de salida inválida.")?;
        let writer = unsafe {
            AVAssetWriter::assetWriterWithURL_fileType_error(&url, constant(AVFileTypeQuickTimeMovie))
        }
        .map_err(|e| format!("No se pudo crear el archivo: {}", e.localizedDescription()))?;

        unsafe {
            // No usar shouldOptimizeForNetworkUse: con fragmentos, reescribir el
            // archivo al cerrarlo falla ("The operation could not be completed").
            writer.setMovieFragmentInterval(CMTime::with_seconds(FRAGMENT_SECONDS, 600));
        }

        let video = new_input(constant(unsafe { AVMediaTypeVideo }), &video_settings(width, height, fps));
        let system_audio = system_audio
            .then(|| AudioTrack::new(2, SYSTEM_AUDIO_BITRATE, "audio del sistema"))
            .transpose()?;
        let microphone =
            microphone.then(|| AudioTrack::new(1, MICROPHONE_BITRATE, "micrófono")).transpose()?;

        let inputs = [Some(&video), system_audio.as_ref().map(|t| &t.input), microphone.as_ref().map(|t| &t.input)];
        for input in inputs.into_iter().flatten() {
            unsafe {
                if !writer.canAddInput(input) {
                    return Err("El formato de grabación no es compatible con este equipo.".into());
                }
                writer.addInput(input);
            }
        }

        if !unsafe { writer.startWriting() } {
            return Err(format!("No se pudo empezar a escribir: {}", writer_error(&writer)));
        }

        Ok(Self {
            writer,
            video,
            system_audio,
            microphone,
            session_started: false,
            session_end: None,
            done: false,
            first_failure: None,
        })
    }

    fn append(&mut self, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
        if self.done || unsafe { self.writer.status() } != AVAssetWriterStatus::Writing {
            return;
        }
        // SAFETY: es el mismo CMSampleBufferRef, prestado mientras dura la llamada.
        let raw = unsafe { &*(sample.as_ptr() as *const RawSampleBuffer) };

        if kind == SCStreamOutputType::Screen {
            // Los cuadros "Idle" no traen imagen: la pantalla no cambió.
            if sample.frame_status() != Some(SCFrameStatus::Complete) {
                return;
            }
            let pts = unsafe { raw.presentation_time_stamp() };
            let seconds = unsafe { pts.seconds() };
            if !self.session_started {
                unsafe { self.writer.startSessionAtSourceTime(pts) };
                self.session_started = true;
                for track in [self.system_audio.as_mut(), self.microphone.as_mut()].into_iter().flatten() {
                    track.conformer.start_at(seconds);
                }
            }
            if unsafe { self.video.isReadyForMoreMediaData() } {
                if unsafe { self.video.appendSampleBuffer(raw) } {
                    let duration = unsafe { raw.duration() };
                    let frame = if unsafe { duration.seconds() } > 0.0 { duration } else { unsafe { CMTime::new(1, 60) } };
                    self.session_end = Some(unsafe { pts.add(frame) });
                } else {
                    self.note_failure("video", String::new());
                }
            }
            self.pad_audio(seconds);
            return;
        }

        // El audio que llega antes del primer cuadro se descarta.
        if !self.session_started {
            return;
        }
        let track = match kind {
            SCStreamOutputType::Audio => self.system_audio.as_mut(),
            _ => self.microphone.as_mut(),
        };
        let Some(track) = track else { return };
        let Some(converted) = track.conformer.convert(sample) else { return };
        if !track.append(&converted) {
            let (label, format) = (track.label, track.conformer.source_format.clone());
            self.note_failure(label, format);
        }
    }

    /// Mantiene cada pista de audio a la par del video rellenando con
    /// silencio cuando no llega sonido: un MP4 fragmentado con fragmentos sin
    /// audio hace fallar al codificador, y además se perdería la sincronía.
    fn pad_audio(&mut self, video_seconds: f64) {
        const MAX_AUDIO_LAG_SECONDS: f64 = 0.5;
        let mut failed = None;
        for track in [self.system_audio.as_mut(), self.microphone.as_mut()].into_iter().flatten() {
            while let Some(silence) = track.conformer.silence_until(video_seconds - MAX_AUDIO_LAG_SECONDS) {
                if !track.append(&silence) {
                    failed = Some(track.label);
                    break;
                }
            }
        }
        if let Some(label) = failed {
            self.note_failure(label, "silencio de relleno".into());
        }
    }

    fn note_failure(&mut self, track: &str, source_format: String) {
        if self.first_failure.is_none() {
            let format = if source_format.is_empty() { String::new() } else { format!(" (entrada: {source_format})") };
            self.first_failure = Some(format!("pista de {track}{format}: {}", writer_error(&self.writer)));
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        if !self.session_started {
            self.cancel();
            return Err("No se recibió ninguna imagen de la fuente; no se guardó el archivo.".into());
        }

        let inputs = [
            Some(&self.video),
            self.system_audio.as_ref().map(|t| &t.input),
            self.microphone.as_ref().map(|t| &t.input),
        ];
        for input in inputs.into_iter().flatten() {
            unsafe { input.markAsFinished() };
        }
        // Sin un fin explícito, las pistas de audio terminan en otro momento que
        // el video y el cierre del MP4 fragmentado falla a veces (-16341).
        if let Some(end) = self.session_end {
            unsafe { self.writer.endSessionAtSourceTime(end) };
        }

        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move || {
            let _ = tx.send(());
        });
        unsafe { self.writer.finishWritingWithCompletionHandler(&handler) };
        rx.recv_timeout(Duration::from_secs(120))
            .map_err(|_| "El archivo tardó demasiado en cerrarse.")?;

        match unsafe { self.writer.status() } {
            AVAssetWriterStatus::Completed => Ok(()),
            _ => Err(format!(
                "La grabación falló en la {}",
                self.first_failure.clone().unwrap_or_else(|| writer_error(&self.writer))
            )),
        }
    }

    fn cancel(&mut self) {
        self.done = true;
        unsafe { self.writer.cancelWriting() };
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

fn shareable_content() -> Result<SCShareableContent, String> {
    // ScreenCaptureKit no muestra el aviso del sistema: sin esto, una app sin
    // permiso ni siquiera aparece en Ajustes para poder activarla.
    unsafe {
        if !CGPreflightScreenCaptureAccess() {
            CGRequestScreenCaptureAccess();
        }
    }
    SCShareableContent::get()
        .map_err(|e| format!("No hay acceso a la pantalla ({e}). {PERMISSION_HINT}"))
}

fn build_filter(
    content: &SCShareableContent,
    options: &RecordOptions,
) -> Result<(SCContentFilter, u32, u32), String> {
    let filter = match options.source_kind {
        SourceKind::Display => {
            let display = content
                .displays()
                .into_iter()
                .find(|d| d.display_id() == options.source_id)
                .ok_or("La pantalla elegida ya no está conectada.")?;
            let own_pid = std::process::id() as i32;
            let own_app = content.applications().into_iter().find(|a| a.process_id() == own_pid);
            let builder = SCContentFilter::create().with_display(&display);
            match (&own_app, options.exclude_self) {
                (Some(app), true) => builder.with_excluding_applications(&[app], &[]),
                _ => builder.with_excluding_windows(&[]),
            }
            .build()
        }
        SourceKind::Window => {
            let window = content
                .windows()
                .into_iter()
                .find(|w| w.window_id() == options.source_id)
                .ok_or("La ventana elegida ya no existe.")?;
            SCContentFilter::create().with_window(&window).build()
        }
    }
    .map_err(sc_error)?;

    let (width, height) = filter_pixels(&filter);
    Ok((filter, width, height))
}

fn display_pixels(display: &SCDisplay) -> Result<(u32, u32), String> {
    let filter = SCContentFilter::create()
        .with_display(display)
        .with_excluding_windows(&[])
        .build()
        .map_err(sc_error)?;
    Ok(filter_pixels(&filter))
}

fn window_pixels(window: &SCWindow) -> Result<(u32, u32), String> {
    let filter = SCContentFilter::create().with_window(window).build().map_err(sc_error)?;
    Ok(filter_pixels(&filter))
}

/// Tamaño real en píxeles de lo que captura el filtro, redondeado a par
/// porque los codificadores de video lo exigen.
fn filter_pixels(filter: &SCContentFilter) -> (u32, u32) {
    let rect = filter.content_rect();
    let scale = f64::from(filter.point_pixel_scale().max(1.0));
    let even = |v: f64| ((v * scale).round() as u32).max(2) & !1;
    (even(rect.size.width), even(rect.size.height))
}

fn video_settings(width: u32, height: u32, fps: u32) -> Retained<NSDictionary<NSString, AnyObject>> {
    let use_hevc = width > H264_MAX.0 || height > H264_MAX.1;
    // ~0,12 bits por píxel: texto nítido en tutoriales sin archivos gigantes.
    let bitrate = (f64::from(width) * f64::from(height) * f64::from(fps) * 0.12)
        .clamp(6_000_000.0, 80_000_000.0) as i64;

    let bitrate = NSNumber::new_i64(bitrate);
    let fps_num = NSNumber::new_u32(fps);
    let keyframes = NSNumber::new_u32(fps * 2);
    let mut keys: Vec<&NSString> = vec![
        constant(unsafe { AVVideoAverageBitRateKey }),
        constant(unsafe { AVVideoExpectedSourceFrameRateKey }),
        constant(unsafe { AVVideoMaxKeyFrameIntervalKey }),
    ];
    let mut values: Vec<&AnyObject> = vec![&bitrate, &fps_num, &keyframes];
    if !use_hevc {
        keys.push(constant(unsafe { AVVideoProfileLevelKey }));
        values.push(constant(unsafe { AVVideoProfileLevelH264HighAutoLevel }));
    }
    let compression = NSDictionary::from_slices(&keys, &values);

    let codec = if use_hevc { constant(unsafe { AVVideoCodecTypeHEVC }) } else { constant(unsafe { AVVideoCodecTypeH264 }) };
    let width = NSNumber::new_u32(width);
    let height = NSNumber::new_u32(height);
    NSDictionary::from_slices(
        &[
            constant(unsafe { AVVideoCodecKey }),
            constant(unsafe { AVVideoWidthKey }),
            constant(unsafe { AVVideoHeightKey }),
            constant(unsafe { AVVideoCompressionPropertiesKey }),
        ],
        &[codec as &AnyObject, &width, &height, &compression],
    )
}

fn audio_settings(channels: u32, bitrate: i32) -> Retained<NSDictionary<NSString, AnyObject>> {
    // Claves de AVAudioSettings.h (AVFAudio); su valor es el propio nombre.
    const K_AUDIO_FORMAT_MPEG4_AAC: u32 = u32::from_be_bytes(*b"aac ");
    let format = NSNumber::new_u32(K_AUDIO_FORMAT_MPEG4_AAC);
    let rate = NSNumber::new_f64(f64::from(SAMPLE_RATE));
    let channels = NSNumber::new_u32(channels);
    let bitrate = NSNumber::new_i32(bitrate);
    NSDictionary::from_slices(
        &[
            &*NSString::from_str("AVFormatIDKey"),
            &*NSString::from_str("AVSampleRateKey"),
            &*NSString::from_str("AVNumberOfChannelsKey"),
            &*NSString::from_str("AVEncoderBitRateKey"),
        ],
        &[&*format as &AnyObject, &rate, &channels, &bitrate],
    )
}

fn new_input(
    media_type: &NSString,
    settings: &NSDictionary<NSString, AnyObject>,
) -> Retained<AVAssetWriterInput> {
    unsafe {
        let input = AVAssetWriterInput::initWithMediaType_outputSettings(
            AVAssetWriterInput::alloc(),
            media_type,
            Some(settings),
        );
        input.setExpectsMediaDataInRealTime(true);
        input
    }
}

/// ScreenCaptureKit no pide el permiso de micrófono: si falta, falla con un
/// mensaje engañoso sobre la pantalla. Por eso se pide aquí antes de empezar.
fn ensure_microphone_access() -> Result<(), String> {
    const DENIED: &str = "No hay permiso para usar el micrófono. Actívalo en Ajustes del Sistema → \
         Privacidad y seguridad → Micrófono, o graba sin micrófono.";
    let audio = constant(unsafe { AVMediaTypeAudio });
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
        AVAuthorizationStatus::Authorized => Ok(()),
        AVAuthorizationStatus::NotDetermined => {
            let (tx, rx) = mpsc::channel();
            let handler = RcBlock::new(move |granted: Bool| {
                let _ = tx.send(granted.as_bool());
            });
            unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler) };
            // Espera a que la persona responda el aviso del sistema.
            match rx.recv_timeout(Duration::from_secs(300)) {
                Ok(true) => Ok(()),
                _ => Err(DENIED.into()),
            }
        }
        _ => Err(DENIED.into()),
    }
}

fn supports_microphone() -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 15,
        minorVersion: 0,
        patchVersion: 0,
    })
}

fn writer_error(writer: &AVAssetWriter) -> String {
    unsafe { writer.error() }
        .map(|e| describe_error(&e))
        .unwrap_or_else(|| "error desconocido".into())
}

/// Mensaje, dominio y código del error y de sus errores de fondo, que es
/// donde AVFoundation deja la causa real.
fn describe_error(error: &NSError) -> String {
    let mut text = format!("{} ({} {})", error.localizedDescription(), error.domain(), error.code());
    let key = unsafe { NSUnderlyingErrorKey };
    if let Some(inner) = error.userInfo().objectForKey(key).and_then(|o| o.downcast::<NSError>().ok()) {
        text.push_str(" ← ");
        text.push_str(&describe_error(&inner));
    }
    text
}

fn constant(value: Option<&'static NSString>) -> &'static NSString {
    value.expect("falta una constante de AVFoundation")
}

fn sc_error(e: SCError) -> String {
    e.to_string()
}
