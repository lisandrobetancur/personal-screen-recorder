// `i18n` primero: su macro `tr!` debe estar definida antes de los demás módulos.
#[macro_use]
pub mod i18n;
pub mod capture;
mod settings;

use std::path::PathBuf;
use std::sync::Mutex;

use capture::health::HealthReport;
use capture::{Microphone, RecordOptions, Recording, Source};
use settings::Settings;
use tauri::{AppHandle, Manager, State};

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
    Ok(path)
}

/// Señales de grabación vacía (silencio digital, imagen negra); None si no se graba.
#[tauri::command]
fn recording_health(state: State<'_, AppState>) -> Option<HealthReport> {
    state.recording.lock().ok()?.as_ref().map(Recording::health)
}

#[tauri::command]
async fn stop_recording(state: State<'_, AppState>) -> Result<PathBuf, String> {
    let recording = state
        .recording
        .lock()
        .map_err(|e| e.to_string())?
        .take()
        .ok_or(tr!("No hay ninguna grabación en curso.", "No recording in progress."))?;
    recording.stop()
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
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_language,
            list_sources,
            list_microphones,
            get_output_dir,
            set_output_dir,
            start_recording,
            stop_recording,
            recording_health
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
