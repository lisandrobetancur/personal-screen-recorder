// `i18n` primero: su macro `tr!` debe estar definida antes de los demás módulos.
#[macro_use]
pub mod i18n;
pub mod capture;
mod settings;
mod tray;

use std::path::PathBuf;
use std::sync::Mutex;

use capture::health::HealthReport;
use capture::{Microphone, RecordOptions, Recording, Source};
use settings::Settings;
use tauri::{AppHandle, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<Recording>>,
}

// Los comandos son `async` para que corran fuera del hilo principal:
// ScreenCaptureKit responde en colas propias y bloquearía la interfaz.

/// Idioma de la interfaz según el sistema ("es" o "en").
#[tauri::command]
fn get_language() -> &'static str {
    i18n::code()
}

/// Atajo global para grabar y detener, como se muestra en la interfaz.
#[tauri::command]
fn get_shortcut() -> &'static str {
    tray::shortcut_label()
}

#[tauri::command]
async fn list_sources() -> Result<Vec<Source>, String> {
    capture::list_sources()
}

#[tauri::command]
async fn list_microphones() -> Vec<Microphone> {
    capture::list_microphones()
}

fn settings_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("settings.json"))
}

fn load_settings(app: &AppHandle) -> Settings {
    settings_file(app).map(|f| Settings::load(&f)).unwrap_or_default()
}

/// Carpeta donde se guardan las grabaciones (la elegida o la de omisión).
#[tauri::command]
fn get_output_dir(app: AppHandle) -> PathBuf {
    load_settings(&app).output_dir()
}

#[tauri::command]
fn set_output_dir(app: AppHandle, path: PathBuf) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err(tr!("Esa carpeta no existe.", "That folder doesn't exist.").into());
    }
    // Comprueba que se pueda escribir ahí antes de guardarla.
    let probe = path.join(".personal-screen-recorder-prueba");
    std::fs::write(&probe, b"").map_err(|_| {
        tr!(
            "No se puede guardar en esa carpeta (sin permiso de escritura).",
            "Can't save to that folder (no write permission).",
        )
    })?;
    let _ = std::fs::remove_file(&probe);

    let mut settings = load_settings(&app);
    settings.output_dir = Some(path.clone());
    settings.save(&settings_file(&app)?)?;
    Ok(path)
}

#[tauri::command]
async fn start_recording(
    options: RecordOptions,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PathBuf, String> {
    let mut slot = state.recording.lock().map_err(|e| e.to_string())?;
    if slot.is_some() {
        return Err(tr!("Ya hay una grabación en curso.", "A recording is already in progress.").into());
    }
    let dir = options.output_dir.clone().unwrap_or_else(|| load_settings(&app).output_dir());
    let path = capture::new_output_path(Some(&dir))?;
    *slot = Some(Recording::start(&options, path.clone())?);
    app.state::<tray::Tray>().set_recording(true);
    Ok(path)
}

/// Señales de grabación vacía (silencio digital, imagen negra); None si no se graba.
#[tauri::command]
fn recording_health(state: State<'_, AppState>) -> Option<HealthReport> {
    state.recording.lock().ok()?.as_ref().map(Recording::health)
}

#[tauri::command]
async fn stop_recording(app: AppHandle, state: State<'_, AppState>) -> Result<PathBuf, String> {
    let recording = state
        .recording
        .lock()
        .map_err(|e| e.to_string())?
        .take()
        .ok_or(tr!("No hay ninguna grabación en curso.", "No recording in progress."))?;
    app.state::<tray::Tray>().set_recording(false);
    recording.stop()
}

fn is_recording(app: &AppHandle) -> bool {
    app.state::<AppState>().recording.lock().is_ok_and(|r| r.is_some())
}

/// Salir desde la barra de menú: si se está grabando, primero se guarda.
pub(crate) fn quit(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let recording = app.state::<AppState>().recording.lock().ok().and_then(|mut r| r.take());
        if let Some(recording) = recording {
            let _ = recording.stop();
        }
        app.exit(0);
    });
}

/// Graba la pantalla número `display` (desde 1) `seconds` segundos sin abrir la interfaz.
/// Sirve para probar el motor con los permisos de la app instalada:
/// `open -W --stdout salida.txt App.app --args --autoprueba 5`.
pub fn self_test(
    seconds: u64,
    display: usize,
    fps: u32,
    system_audio: bool,
    microphone: bool,
) -> Result<PathBuf, String> {
    let display = capture::list_sources()?
        .into_iter()
        .filter(|s| s.kind == capture::SourceKind::Display)
        .nth(display.saturating_sub(1))
        .ok_or(tr!("No existe esa pantalla.", "That display doesn't exist."))?;
    let options = RecordOptions {
        source_kind: display.kind,
        source_id: display.id,
        fps,
        system_audio,
        microphone,
        microphone_id: None,
        show_cursor: true,
        show_clicks: false,
        exclude_self: true,
        output_dir: None,
    };
    let recording = Recording::start(&options, capture::new_output_path(None)?)?;
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    // Lo que detectó la vigilancia de grabación vacía, para revisarlo en las pruebas.
    eprintln!("{:?}", recording.health());
    recording.stop()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        tray::request_toggle(app);
                    }
                })
                .build(),
        )
        .manage(AppState::default())
        .setup(|app| {
            tray::setup(app.handle())?;
            // Si otra app ya usa el atajo, se sigue sin él (queda el ícono).
            if let Err(e) = app.global_shortcut().register(tray::SHORTCUT) {
                eprintln!("No se pudo registrar el atajo {}: {e}", tray::SHORTCUT);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Cerrar la ventana mientras se graba no corta la grabación: solo
            // la oculta (se detiene desde el ícono o con el atajo).
            if let WindowEvent::CloseRequested { api, .. } = event {
                if is_recording(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_language,
            get_shortcut,
            list_sources,
            list_microphones,
            get_output_dir,
            set_output_dir,
            start_recording,
            stop_recording,
            recording_health
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app, event| {
        // Mac: clic en el ícono del Dock con la ventana oculta la vuelve a mostrar.
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            tray::show_window(app);
        }
        let _ = (app, event);
    });
}
