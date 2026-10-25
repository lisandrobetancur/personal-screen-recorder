//! Marcador para sistemas cuyo motor aún no existe (Windows, por ahora).

use std::path::PathBuf;

use super::{Microphone, RecordOptions, Source};

const PENDING: &str = "La grabación en este sistema todavía no está implementada.";

pub fn list_sources() -> Result<Vec<Source>, String> {
    Err(PENDING.into())
}

pub fn list_microphones() -> Vec<Microphone> {
    Vec::new()
}

pub struct Recording;

impl Recording {
    pub fn start(_options: &RecordOptions, _path: PathBuf) -> Result<Self, String> {
        Err(PENDING.into())
    }

    pub fn stop(self) -> Result<PathBuf, String> {
        Err(PENDING.into())
    }
}
