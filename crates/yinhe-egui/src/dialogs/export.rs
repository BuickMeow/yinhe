pub use yinhe_audio::export::ExportProgress;

/// Captured state when an export finishes successfully, used to show the
/// completion dialog with elapsed time, overall speed, and an "open folder" button.
pub(crate) struct ExportCompleted {
    pub file_path: String,
    pub elapsed_secs: f64,
    pub overall_speed: f64,
}
