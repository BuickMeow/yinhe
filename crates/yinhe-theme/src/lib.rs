pub mod base;
#[cfg(feature = "egui")]
pub mod egui_colors;
mod gpu_theme;
pub mod palette;

pub use gpu_theme::{GpuTheme, current_gpu_theme, set_current_gpu_theme};
pub use palette::TRACK_PALETTE;
