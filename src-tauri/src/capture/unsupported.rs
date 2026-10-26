//! Marcador para sistemas sin motor de grabación (todo salvo macOS y Windows).

use std::path::PathBuf;

use super::health::HealthReport;
use super::{Microphone, RecordOptions, Source};

fn pending() -> String {
    tr!(
        "La grabación en este sistema todavía no está implementada.",
        "Recording isn't supported on this system yet.",
    )
    .into()
}

pub fn list_sources() -> Result<Vec<Source>, String> {
    Err(pending())
}

pub fn list_microphones() -> Vec<Microphone> {
    Vec::new()
}

pub struct Recording;

impl Recording {
    pub fn start(_options: &RecordOptions, _path: PathBuf) -> Result<Self, String> {
        Err(pending())
    }

    pub fn health(&self) -> HealthReport {
        super::health::Health::default().report()
    }

    pub fn stop(self) -> Result<PathBuf, String> {
        Err(pending())
    }
}
