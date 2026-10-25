pub mod capture;

use std::path::PathBuf;
use std::sync::Mutex;

use capture::{Microphone, RecordOptions, Recording, Source};
use tauri::State;

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<Recording>>,
}

// Los comandos son `async` para que corran fuera del hilo principal:
// ScreenCaptureKit responde en colas propias y bloquearía la interfaz.

#[tauri::command]
async fn list_sources() -> Result<Vec<Source>, String> {
    capture::list_sources()
}

#[tauri::command]
async fn list_microphones() -> Vec<Microphone> {
    capture::list_microphones()
}

#[tauri::command]
fn default_output_dir() -> PathBuf {
    capture::default_output_dir()
}

#[tauri::command]
async fn start_recording(options: RecordOptions, state: State<'_, AppState>) -> Result<PathBuf, String> {
    let mut slot = state.recording.lock().map_err(|e| e.to_string())?;
    if slot.is_some() {
        return Err("Ya hay una grabación en curso.".into());
    }
    let path = capture::new_output_path(options.output_dir.as_deref())?;
    *slot = Some(Recording::start(&options, path.clone())?);
    Ok(path)
}

#[tauri::command]
async fn stop_recording(state: State<'_, AppState>) -> Result<PathBuf, String> {
    let recording = state
        .recording
        .lock()
        .map_err(|e| e.to_string())?
        .take()
        .ok_or("No hay ninguna grabación en curso.")?;
    recording.stop()
}

/// Graba la primera pantalla `seconds` segundos sin abrir la interfaz.
/// Sirve para probar el motor con los permisos de la app instalada:
/// `open -W --stdout salida.txt App.app --args --autoprueba 5`.
pub fn self_test(seconds: u64, microphone: bool) -> Result<PathBuf, String> {
    let display = capture::list_sources()?
        .into_iter()
        .find(|s| s.kind == capture::SourceKind::Display)
        .ok_or("No hay pantallas.")?;
    let options = RecordOptions {
        source_kind: display.kind,
        source_id: display.id,
        fps: 30,
        system_audio: true,
        microphone,
        microphone_id: None,
        show_cursor: true,
        show_clicks: false,
        exclude_self: true,
        output_dir: None,
    };
    let recording = Recording::start(&options, capture::new_output_path(None)?)?;
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    recording.stop()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            list_sources,
            list_microphones,
            default_output_dir,
            start_recording,
            stop_recording
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
