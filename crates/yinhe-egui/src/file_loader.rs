use rust_i18n::t;
// re-export，让 app/poll.rs 的 use 不变
pub(crate) use yinhe_editor_core::file_loading::LoadResult;
use yinhe_editor_core::progress::SharedProgress;
use yinhe_midi::MidiImportEncoding;

use crate::dialogs::archive_picker::{ArchivePickerState, PasswordPrompt};

/// UI 层薄封装：文件对话框（rfd）与 i18n 文案在这里，
/// 加载逻辑在 yinhe-editor-core 的 `file_loading` 模块（平台无关）。
pub(crate) struct FileLoader {
    core: yinhe_editor_core::file_loading::FileLoader,
    pub archive_picker: Option<ArchivePickerState>,
    pub password_prompt: Option<PasswordPrompt>,
}

impl FileLoader {
    pub fn new(load_progress: SharedProgress) -> Self {
        Self {
            core: yinhe_editor_core::file_loading::FileLoader::new(
                load_progress,
                yinhe_editor_core::file_loading::LoadStageLabels {
                    yin: t!("dialog.loading.yin_stage").to_string(),
                    archive: t!("dialog.loading.archive_stage").to_string(),
                    yin_decompress: t!("dialog.loading.yin_decompress").to_string(),
                    yin_rebuild: t!("dialog.loading.yin_rebuild").to_string(),
                    yin_resort: t!("dialog.loading.yin_resort").to_string(),
                },
            ),
            archive_picker: None,
            password_prompt: None,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.core.is_loading() || self.archive_picker.is_some() || self.password_prompt.is_some()
    }

    pub fn load_progress(&self) -> &SharedProgress {
        self.core.load_progress()
    }

    /// Cancel any in-progress loading. Clears UI dialog state along with core loaders.
    pub fn cancel_loading(&mut self) {
        self.core.cancel_loading();
        self.password_prompt = None;
    }

    /// Show file dialog and start loading in a background thread.
    pub fn pick_file(&mut self, encoding: MidiImportEncoding) {
        if self.is_loading() {
            return;
        }

        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                t!("file_dialog.all_supported").as_ref(),
                &[
                    "mid", "midi", "yin", "zip", "7z", "rar", "lzh", "lha", "tar", "gz", "xz",
                    "tgz", "txz", "tbz", "bz2",
                ],
            )
            .add_filter(t!("file_dialog.midi").as_ref(), &["mid", "midi"])
            .add_filter(t!("file_dialog.yinhe_project").as_ref(), &["yin"])
            .add_filter(
                t!("file_dialog.archive").as_ref(),
                &[
                    "zip", "7z", "rar", "lzh", "lha", "tar", "gz", "xz", "tgz", "txz", "tbz", "bz2",
                ],
            )
            .pick_file()
        {
            let path_str = path.to_string_lossy().to_string();
            self.core.load_path(path_str, encoding);
        }
    }

    /// 加载指定路径的文件（文件对话框 / Finder 打开方式共用）。
    pub fn load_path(&mut self, path_str: String, encoding: MidiImportEncoding) {
        self.core.load_path(path_str, encoding);
    }

    pub fn start_archive(&mut self, path_str: String, password: Option<String>) {
        self.core.start_archive(path_str, password);
    }

    pub fn start_load_from_archive(
        &mut self,
        archive: yinhe_archive::Archive,
        entry: yinhe_archive::ArchiveEntry,
    ) {
        self.core.start_load_from_archive(archive, entry);
    }

    pub fn poll_loading(&mut self) -> LoadResult {
        use yinhe_editor_core::file_loading::LoadResult as Core;
        match self.core.poll_loading() {
            Core::ModelLoaded { path, model } => LoadResult::ModelLoaded { path, model },
            Core::ModelFromYin {
                path,
                model,
                file_name,
                sf,
                mapping,
            } => LoadResult::ModelFromYin {
                path,
                model,
                file_name,
                sf,
                mapping,
            },
            Core::ArchivePickerNeeded { path, rx } => {
                self.archive_picker = Some(ArchivePickerState::Opening { path, rx });
                LoadResult::NotReady
            }
            Core::PasswordNeeded {
                path,
                wrong_password,
            } => {
                self.password_prompt = Some(PasswordPrompt::new(path, wrong_password));
                LoadResult::NotReady
            }
            Core::ArchiveError(msg) => LoadResult::ArchiveError(msg),
            Core::NotReady => LoadResult::NotReady,
        }
    }
}
