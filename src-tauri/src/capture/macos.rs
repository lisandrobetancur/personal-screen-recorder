//! Motor de macOS: ScreenCaptureKit captura pantalla, audio del sistema y
//! micrófono; AVAssetWriter codifica por hardware (VideoToolbox) a MP4.
//!
//! El MP4 se escribe fragmentado cada pocos segundos: si la app se cierra de
//! golpe, el archivo sigue siendo reproducible hasta el último fragmento.

use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaTypeAudio,
    AVMediaTypeVideo, AVVideoAverageBitRateKey, AVVideoCodecKey, AVVideoCodecTypeH264,
    AVVideoCodecTypeHEVC, AVVideoCompressionPropertiesKey, AVVideoExpectedSourceFrameRateKey,
    AVVideoHeightKey, AVVideoMaxKeyFrameIntervalKey, AVVideoProfileLevelH264HighAutoLevel,
    AVVideoProfileLevelKey, AVVideoWidthKey,
};
use objc2_core_media::{CMSampleBuffer as RawSampleBuffer, CMTime};
use objc2_foundation::{NSDictionary, NSNumber, NSOperatingSystemVersion, NSProcessInfo, NSString, NSURL};
use screencapturekit::cm::SCFrameStatus;
use screencapturekit::prelude::*;

use super::{Microphone, RecordOptions, Source, SourceKind};

const SAMPLE_RATE: u32 = 48_000;
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
        Ok(self.path)
    }
}

struct Writer {
    writer: Retained<AVAssetWriter>,
    video: Retained<AVAssetWriterInput>,
    system_audio: Option<Retained<AVAssetWriterInput>>,
    microphone: Option<Retained<AVAssetWriterInput>>,
    session_started: bool,
    done: bool,
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
            AVAssetWriter::assetWriterWithURL_fileType_error(&url, constant(AVFileTypeMPEG4))
        }
        .map_err(|e| format!("No se pudo crear el archivo: {}", e.localizedDescription()))?;

        unsafe {
            // No usar shouldOptimizeForNetworkUse: con fragmentos, reescribir el
            // archivo al cerrarlo falla ("The operation could not be completed").
            writer.setMovieFragmentInterval(CMTime::with_seconds(FRAGMENT_SECONDS, 600));
        }

        let video = new_input(constant(unsafe { AVMediaTypeVideo }), &video_settings(width, height, fps));
        let system_audio = system_audio
            .then(|| new_input(constant(unsafe { AVMediaTypeAudio }), &audio_settings(2, SYSTEM_AUDIO_BITRATE)));
        let microphone = microphone
            .then(|| new_input(constant(unsafe { AVMediaTypeAudio }), &audio_settings(1, MICROPHONE_BITRATE)));

        for input in [Some(&video), system_audio.as_ref(), microphone.as_ref()].into_iter().flatten() {
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

        Ok(Self { writer, video, system_audio, microphone, session_started: false, done: false })
    }

    fn append(&mut self, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
        if self.done || unsafe { self.writer.status() } != AVAssetWriterStatus::Writing {
            return;
        }
        // SAFETY: es el mismo CMSampleBufferRef, prestado mientras dura la llamada.
        let raw = unsafe { &*(sample.as_ptr() as *const RawSampleBuffer) };

        let input = match kind {
            SCStreamOutputType::Screen => {
                // Los cuadros "Idle" no traen imagen: la pantalla no cambió.
                if sample.frame_status() != Some(SCFrameStatus::Complete) {
                    return;
                }
                if !self.session_started {
                    unsafe { self.writer.startSessionAtSourceTime(raw.presentation_time_stamp()) };
                    self.session_started = true;
                }
                &self.video
            }
            // El audio que llega antes del primer cuadro se descarta.
            _ if !self.session_started => return,
            SCStreamOutputType::Audio => match &self.system_audio {
                Some(i) => i,
                None => return,
            },
            SCStreamOutputType::Microphone => match &self.microphone {
                Some(i) => i,
                None => return,
            },
        };

        unsafe {
            if input.isReadyForMoreMediaData() {
                input.appendSampleBuffer(raw);
            }
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

        for input in [Some(&self.video), self.system_audio.as_ref(), self.microphone.as_ref()]
            .into_iter()
            .flatten()
        {
            unsafe { input.markAsFinished() };
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
            _ => Err(format!("La grabación falló: {}", writer_error(&self.writer))),
        }
    }

    fn cancel(&mut self) {
        self.done = true;
        unsafe { self.writer.cancelWriting() };
    }
}

fn shareable_content() -> Result<SCShareableContent, String> {
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

fn supports_microphone() -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 15,
        minorVersion: 0,
        patchVersion: 0,
    })
}

fn writer_error(writer: &AVAssetWriter) -> String {
    unsafe { writer.error() }
        // El detalle (dominio, código y error de fondo) sirve para diagnosticar.
        .map(|e| format!("{} [{e:?}]", e.localizedDescription()))
        .unwrap_or_else(|| "error desconocido".into())
}

fn constant(value: Option<&'static NSString>) -> &'static NSString {
    value.expect("falta una constante de AVFoundation")
}

fn sc_error(e: SCError) -> String {
    e.to_string()
}
