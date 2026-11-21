//! Yinhe 移动端平台无关核心。
//!
//! 这里放 Flutter/Android/iOS/OHOS 共用的移动编辑逻辑：
//! - `scene`：文档快照与 Dart 渲染场景投影；
//! - `edit`：触屏手势、几何、预览与提交；
//! - `audio`：移动音频会话（CPAL + yinhe-audio）；
//! - `doc_job` / `autosave` / `midi_import` / `midi_export` / `library`：
//!   后台文档任务与工程 IO；
//! - `selection_materializer` / `inertia` / `velocity_cache` / `theme_colors`：辅助状态。
//!
//! 平台胶水（JNI/NAPI/objc、纹理/surface、FFI 导出）留在 `yinhe-flutter`。

pub mod audio;
pub mod autosave;
pub mod doc_job;
pub mod edit;
pub mod inertia;
pub mod library;
pub mod midi_export;
pub mod midi_import;
pub mod midi_names;
pub mod scene;
pub mod selection_materializer;
pub mod soundfont_archive;
pub mod task_center;
pub mod theme_colors;
pub mod velocity_cache;
pub mod viewport;

pub use viewport::{effective_visible, logical_size, set_logical_size};
