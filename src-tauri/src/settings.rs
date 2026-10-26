//! Preferencias que se conservan entre sesiones (por ahora, la carpeta de
//! grabaciones), guardadas como JSON en la carpeta de configuración de la app.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub output_dir: Option<PathBuf>,
}

impl Settings {
    /// Lee las preferencias; si el archivo no existe o está dañado, usa las de omisión.
    pub fn load(file: &Path) -> Self {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> Result<(), String> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(file, text).map_err(|e| tr!(format!("No se pudo guardar la preferencia: {e}"), format!("Couldn't save the setting: {e}")))
    }

    /// Carpeta elegida si todavía existe; si no, la de omisión.
    pub fn output_dir(&self) -> PathBuf {
        self.output_dir
            .clone()
            .filter(|d| d.is_dir())
            .unwrap_or_else(crate::capture::default_output_dir)
    }
}
