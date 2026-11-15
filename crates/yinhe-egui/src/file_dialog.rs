//! 文件对话框统一入口：按类别套用设置里的默认起始目录。
//!
//! 未配置目录时不调用 `set_directory`，继续跟随系统上次打开的目录。

use std::path::Path;

use yinhe_editor_core::audio_settings::{AudioSettings, FileDialogKind};

/// 按类别创建对话框。
pub(crate) fn dialog(settings: &AudioSettings, kind: FileDialogKind) -> rfd::FileDialog {
    with_dir(settings.dialog_dir(kind))
}

/// 视口闭包里拿不到整份设置时，直接传入已解析的目录。
///
/// 目录不存在（移动硬盘拔掉 / 文件夹被删）时忽略，避免对话框卡在无效路径。
pub(crate) fn with_dir(dir: Option<&str>) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new();
    if let Some(path) = dir.map(Path::new).filter(|p| p.is_dir()) {
        dialog = dialog.set_directory(path);
    }
    dialog
}
