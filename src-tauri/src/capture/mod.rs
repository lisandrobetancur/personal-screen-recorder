//! Tipos comunes de captura y selección del motor de cada sistema.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod health;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_audio;
#[cfg(target_os = "macos")]
pub use macos::{list_microphones, list_sources, Recording};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{list_microphones, list_sources, Recording};

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use unsupported::{list_microphones, list_sources, Recording};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Display,
    Window,
}

/// Algo que se puede grabar: una pantalla o una ventana.
#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub kind: SourceKind,
    pub id: u32,
    pub name: String,
    /// Tamaño en píxeles reales (no puntos).
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Microphone {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordOptions {
    pub source_kind: SourceKind,
    pub source_id: u32,
    pub fps: u32,
    pub system_audio: bool,
    pub microphone: bool,
    /// `None` usa el micrófono predeterminado del sistema.
    pub microphone_id: Option<String>,
    pub show_cursor: bool,
    pub show_clicks: bool,
    /// Oculta las ventanas de esta app al grabar una pantalla.
    pub exclude_self: bool,
    /// `None` usa la carpeta de videos del usuario.
    pub output_dir: Option<PathBuf>,
}

/// Carpeta de salida por omisión: `~/Movies/Personal Screen Recorder` en Mac,
/// `Videos\Personal Screen Recorder` en Windows.
pub fn default_output_dir() -> PathBuf {
    dirs::video_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Personal Screen Recorder")
}

/// Extensión del archivo mientras se graba. En Mac se graba a .mov y al
/// terminar se convierte a .mp4; en Windows se escribe el .mp4 directamente.
#[cfg(target_os = "macos")]
const RECORDING_EXTENSION: &str = "mov";
#[cfg(not(target_os = "macos"))]
const RECORDING_EXTENSION: &str = "mp4";

/// Ruta nueva y única para una grabación dentro de `dir`.
pub fn new_output_path(dir: Option<&Path>) -> Result<PathBuf, String> {
    let dir = dir.map(Path::to_path_buf).unwrap_or_else(default_output_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| {
        tr!(
            format!("No se pudo crear la carpeta {}: {e}", dir.display()),
            format!("Couldn't create the folder {}: {e}", dir.display()),
        )
    })?;
    let stamp = chrono::Local::now().format("%Y-%m-%d %H.%M.%S");
    let prefix = tr!("Grabación", "Recording");
    let name = |suffix: &str| dir.join(format!("{prefix} {stamp}{suffix}.{RECORDING_EXTENSION}"));
    let mut path = name("");
    let mut n = 2;
    // Ni el archivo de trabajo ni el .mp4 final deben existir ya.
    while path.exists() || path.with_extension("mp4").exists() {
        path = name(&format!(" ({n})"));
        n += 1;
    }
    Ok(path)
}
