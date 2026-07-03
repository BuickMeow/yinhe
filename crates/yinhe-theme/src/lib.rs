pub mod palette;
mod gpu_theme;
#[cfg(feature = "egui")]
pub mod egui_colors;

pub use gpu_theme::GpuTheme;
pub use palette::TRACK_PALETTE;
