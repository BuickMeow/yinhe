use std::sync::{Arc, Mutex};

/// Shared export progress state, updated from the background thread.
#[derive(Clone)]
pub(crate) struct ExportProgress {
    pub visible: bool,
    pub progress: f32,
    pub status: String,
}

impl ExportProgress {
    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            visible: false,
            progress: 0.0,
            status: String::new(),
        }))
    }
}
