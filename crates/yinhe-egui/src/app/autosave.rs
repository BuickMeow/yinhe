//! 自动保存：定时把脏文档备份到 `<config>/yinhe/autosave/`。
//!
//! 设计要点：
//! - 只备份脏文档，不改变脏标记与 `file_path`（与"用户保存"语义分离）
//! - `manifest.json` 记录「文档 ↔ autosave 文件 ↔ 原路径」映射，用于崩溃残留
//!   检测、启动恢复、以及 GPU 设备丢失后的自动重启恢复
//! - 正常保存/关闭文档时删除对应备份；正常退出时清空整个目录
//! - 写盘发生在后台线程（只持有 `SaveSnapshot`），主线程做同步准备与结果回收
//! - 恢复是顺序的（`FileLoader` 一次只加载一个），加载完成后绑定原路径并删除备份

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::app::actions::SaveSnapshot;

/// device lost 自动重启时，旧进程 spawn 新进程所设置的环境变量；
/// 新进程读到残留 manifest 后无需询问直接恢复。
pub(crate) const RECOVER_ENV: &str = "YINHE_AUTOSAVE_RECOVER";

/// manifest 中一条自动保存记录。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AutoSaveEntry {
    /// 文档会话 id（autosave 文件名来源）。
    pub doc_id: u64,
    /// 备份文件绝对路径（`backup = false` 时为原文件路径）。
    pub file: String,
    /// 原文件路径（未保存过的新文档为 None）。
    pub original: Option<String>,
    /// 标签页显示名。
    pub name: String,
    /// 写盘时间（unix 秒，供恢复界面显示）。
    pub saved_at: u64,
    /// 是否为本应用写出的备份文件（false = 直接重开原文件，恢复后不删除）。
    #[serde(default = "default_true")]
    pub backup: bool,
}

fn default_true() -> bool {
    true
}

/// autosave 目录的 manifest。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Manifest {
    /// device lost 自动重启：新进程启动后无需询问直接恢复。
    #[serde(default)]
    pub auto_recover: bool,
    #[serde(default)]
    pub entries: Vec<AutoSaveEntry>,
    /// 直接重开（未修改文档的原路径，仅自动恢复用）。
    #[serde(default)]
    pub reopen: Vec<String>,
}

/// autosave 目录（不存在时按需创建）。
pub(crate) fn autosave_dir() -> PathBuf {
    yinhe_editor_core::paths::app_config_dir().join("autosave")
}

fn manifest_path() -> PathBuf {
    autosave_dir().join("manifest.json")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 读取磁盘上的残留 manifest。损坏/缺失时返回默认值。
pub(crate) fn load_manifest() -> Manifest {
    let Ok(bytes) = std::fs::read(manifest_path()) else {
        return Manifest::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// 重写 manifest；内容为空时删除文件。
pub(crate) fn save_manifest(manifest: &Manifest) {
    let dir = autosave_dir();
    let _ = std::fs::create_dir_all(&dir);
    if manifest.entries.is_empty() && manifest.reopen.is_empty() && !manifest.auto_recover {
        let _ = std::fs::remove_file(manifest_path());
        return;
    }
    match serde_json::to_vec_pretty(manifest) {
        Ok(bytes) => {
            let _ = std::fs::write(manifest_path(), bytes);
        }
        Err(e) => tracing::warn!("[autosave] manifest 序列化失败: {e}"),
    }
}

/// 删除一条记录的备份文件（`backup = false` 时是原文件，绝不删除）。
pub(crate) fn delete_entry_file(entry: &AutoSaveEntry) {
    if !entry.backup {
        return;
    }
    if let Err(e) = std::fs::remove_file(&entry.file)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("[autosave] 删除备份失败 {}: {e}", entry.file);
    }
}

/// 自动保存运行时状态（挂在 App 上）。
#[derive(Default)]
pub(crate) struct AutoSaveState {
    /// 磁盘 manifest 的内存镜像（entries 用于清理；auto_recover 消费后清零）。
    pub manifest: Manifest,
    /// 启动时发现的残留（恢复弹窗数据源）。
    pub recovery: Option<Vec<AutoSaveEntry>>,
    /// 残留是否已处理（自动恢复或已弹窗），避免重复触发。
    recovery_handled: bool,
    /// 残留是否为 device lost 自动重启（无需询问）。
    recovery_auto: bool,
    /// 自动重启时待直接重开的原路径。
    recovery_reopen: Vec<String>,
    /// 是否显示恢复询问弹窗。
    pub show_recovery_dialog: bool,
    /// 顺序恢复队列（FileLoader 一次只能加载一个）。
    restore_queue: VecDeque<AutoSaveEntry>,
    /// 正在加载的恢复文件（autosave 路径 → 记录）。
    pub pending_restore: HashMap<String, AutoSaveEntry>,
    /// 恢复流程进行中。
    pub restore_in_flight: bool,
    /// 后台自动保存批次进行中。
    in_flight: bool,
    /// 上次批次完成时刻。
    last_run: Option<Instant>,
    rx: Option<mpsc::Receiver<Vec<AutoSaveEntry>>>,

    // ── device lost 自动重启 ──
    /// 保全流程已启动。
    pub lost_started: bool,
    /// 保全/重启失败（回退到手动重启弹窗）。
    pub lost_failed: bool,
    lost_rx: Option<mpsc::Receiver<bool>>,
}

impl AutoSaveState {
    /// 启动时初始化：读取残留 manifest。
    pub fn new() -> Self {
        let manifest = load_manifest();
        let auto_recover = manifest.auto_recover;
        let mut state = Self {
            manifest,
            ..Default::default()
        };
        if !state.manifest.entries.is_empty() || !state.manifest.reopen.is_empty() {
            state.recovery = Some(state.manifest.entries.clone());
            state.recovery_auto = auto_recover;
            state.recovery_reopen = state.manifest.reopen.clone();
        }
        state
    }

    fn interval_due(&self, interval_secs: u64) -> bool {
        let interval = Duration::from_secs(interval_secs.clamp(10, 24 * 3600));
        self.last_run.is_none_or(|t| t.elapsed() >= interval)
    }
}

impl App {
    /// 每帧调用：回收后台结果、按间隔触发自动保存、推进恢复流程。
    pub(crate) fn poll_autosave(&mut self) {
        self.poll_autosave_batch_result();
        self.poll_autosave_trigger();
        self.poll_recovery_start();
        self.pump_restore_queue();
    }

    fn poll_autosave_batch_result(&mut self) {
        let recv = self
            .autosave
            .rx
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(entries) => Some(entries),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("[autosave] 后台线程异常退出");
                    Some(Vec::new())
                }
            });
        let Some(new_entries) = recv else {
            return;
        };
        self.autosave.rx = None;
        self.autosave.in_flight = false;
        self.autosave.last_run = Some(Instant::now());
        // 同一文档的旧备份文件（路径可能因 doc_id 复用而不同）先删再合并
        for e in &new_entries {
            let stale: Vec<AutoSaveEntry> = self
                .autosave
                .manifest
                .entries
                .iter()
                .filter(|o| o.doc_id == e.doc_id && o.file != e.file)
                .cloned()
                .collect();
            for old in &stale {
                delete_entry_file(old);
            }
            self.autosave
                .manifest
                .entries
                .retain(|o| o.doc_id != e.doc_id);
            self.autosave.manifest.entries.push(e.clone());
        }
        save_manifest(&self.autosave.manifest);
    }

    fn poll_autosave_trigger(&mut self) {
        if self.autosave.in_flight || !self.audio_settings.auto_save_enabled {
            return;
        }
        if !self
            .autosave
            .interval_due(self.audio_settings.auto_save_interval_secs)
        {
            return;
        }
        self.run_autosave_batch();
    }

    /// 触发一轮自动保存（只处理脏文档；无脏文档时仅重置计时）。
    pub(crate) fn run_autosave_batch(&mut self) {
        self.autosave.last_run = Some(Instant::now());
        if self.autosave.in_flight {
            return;
        }
        let dirty: Vec<usize> = self
            .workspace
            .documents
            .iter()
            .enumerate()
            .filter(|(_, d)| d.is_dirty())
            .map(|(i, _)| i)
            .collect();
        if dirty.is_empty() {
            return;
        }
        let jobs: Vec<AutoSaveJob> = dirty
            .into_iter()
            .map(|idx| self.autosave_job(idx))
            .collect();
        self.spawn_autosave_thread(jobs, None);
    }

    /// 为一个文档构造后台保存任务。
    fn autosave_job(&mut self, idx: usize) -> AutoSaveJob {
        let snapshot = self.take_save_snapshot(idx);
        let doc = &self.workspace.documents[idx];
        let path = autosave_dir().join(format!("doc_{}.yin", doc.doc_id));
        AutoSaveJob {
            doc_id: doc.doc_id,
            path,
            original: doc.file_path.clone(),
            name: doc.file_name.clone(),
            snapshot,
        }
    }

    /// 后台线程顺序保存任务；`lost` 为 Some 时走 device lost 保全路径
    /// （写 auto_recover manifest，结果用 lost_rx 通知）。
    fn spawn_autosave_thread(&mut self, jobs: Vec<AutoSaveJob>, lost: Option<LostContext>) {
        match lost {
            Some(lost) => {
                let (tx, rx) = mpsc::channel::<bool>();
                std::thread::spawn(move || {
                    let (entries, all_ok) = save_jobs(jobs);
                    // device lost：写自动恢复 manifest，通知主线程 spawn 新进程。
                    // 任一备份失败时不自动重启（回退手动弹窗，避免半套数据）。
                    let ok = all_ok && (!entries.is_empty() || !lost.reopen.is_empty());
                    save_manifest(&Manifest {
                        auto_recover: true,
                        entries,
                        reopen: lost.reopen,
                    });
                    let _ = tx.send(ok);
                });
                self.autosave.lost_rx = Some(rx);
            }
            None => {
                let (tx, rx) = mpsc::channel::<Vec<AutoSaveEntry>>();
                std::thread::spawn(move || {
                    let (entries, _) = save_jobs(jobs);
                    let _ = tx.send(entries);
                });
                self.autosave.in_flight = true;
                self.autosave.rx = Some(rx);
            }
        }
    }

    /// 删除某文档的自动保存备份（用户正常保存成功/关闭文档时调用）。
    pub(crate) fn discard_autosave_for(&mut self, doc_id: u64) {
        let Some(pos) = self
            .autosave
            .manifest
            .entries
            .iter()
            .position(|e| e.doc_id == doc_id)
        else {
            return;
        };
        let entry = self.autosave.manifest.entries.remove(pos);
        delete_entry_file(&entry);
        save_manifest(&self.autosave.manifest);
    }

    /// 清空全部自动保存（正常退出时调用）。
    pub(crate) fn clear_autosave(&mut self) {
        let entries = std::mem::take(&mut self.autosave.manifest.entries);
        for e in &entries {
            delete_entry_file(e);
        }
        self.autosave.manifest.auto_recover = false;
        self.autosave.manifest.reopen.clear();
        save_manifest(&self.autosave.manifest);
    }

    // ── 启动恢复 ──

    /// 首次 poll 时处理启动残留：自动重启直接恢复，否则弹询问窗。
    fn poll_recovery_start(&mut self) {
        if self.autosave.recovery_handled {
            return;
        }
        let Some(entries) = self.autosave.recovery.clone() else {
            return;
        };
        self.autosave.recovery_handled = true;
        if self.autosave.recovery_auto {
            let reopen = std::mem::take(&mut self.autosave.recovery_reopen);
            let mut queue: Vec<AutoSaveEntry> = entries;
            for path in reopen {
                let name = std::path::Path::new(&path)
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                queue.push(AutoSaveEntry {
                    doc_id: 0,
                    file: path.clone(),
                    original: Some(path),
                    name,
                    saved_at: 0,
                    backup: false,
                });
            }
            self.restore_autosave_entries(queue);
        } else {
            self.autosave.show_recovery_dialog = true;
        }
    }

    /// 开始恢复给定记录（顺序加载）。
    pub(crate) fn restore_autosave_entries(&mut self, entries: Vec<AutoSaveEntry>) {
        self.autosave.recovery = None;
        self.autosave.show_recovery_dialog = false;
        self.autosave.restore_in_flight = true;
        self.autosave.restore_queue.extend(entries);
        self.pump_restore_queue();
    }

    /// 丢弃残留（恢复弹窗选"丢弃"）：删备份文件、清 manifest。
    pub(crate) fn discard_recovery(&mut self, entries: &[AutoSaveEntry]) {
        for e in entries {
            delete_entry_file(e);
        }
        self.autosave.manifest.entries.clear();
        self.autosave.manifest.auto_recover = false;
        self.autosave.manifest.reopen.clear();
        save_manifest(&self.autosave.manifest);
        self.autosave.recovery = None;
        self.autosave.show_recovery_dialog = false;
    }

    /// 队列推进：空闲时加载下一个恢复文件。
    fn pump_restore_queue(&mut self) {
        if self.file_loader.is_loading() {
            return;
        }
        let Some(entry) = self.autosave.restore_queue.pop_front() else {
            if self.autosave.restore_in_flight {
                self.autosave.restore_in_flight = false;
            }
            return;
        };
        self.autosave
            .pending_restore
            .insert(entry.file.clone(), entry.clone());
        self.file_loader
            .load_path(entry.file.clone(), self.audio_settings.midi_import_encoding);
    }

    /// 加载完成后调用：绑定恢复文档的原路径/名称，删除备份，继续队列。
    /// 返回该加载是否为恢复流程（调用方据此跳过"最近使用"记录）。
    pub(crate) fn finish_restore_one(&mut self, path: &str) -> bool {
        let Some(entry) = self.autosave.pending_restore.remove(path) else {
            return false;
        };
        if let Some(idx) = self.workspace.active_doc {
            let doc = &mut self.workspace.documents[idx];
            if let Some(orig) = &entry.original {
                doc.file_path = Some(orig.clone());
                doc.file_name = entry.name.clone();
            }
            // 恢复内容来自备份而非原文件：保持"未保存"状态提醒用户落盘。
            doc.mark_loaded();
        }
        delete_entry_file(&entry);
        self.autosave
            .manifest
            .entries
            .retain(|e| e.doc_id != entry.doc_id);
        self.autosave.manifest.auto_recover = false;
        save_manifest(&self.autosave.manifest);
        self.pump_restore_queue();
        true
    }

    // ── device lost 自动重启 ──

    /// device lost 处理：保全未保存文档 → spawn 新进程自动恢复 → 退出。
    /// 任一环节失败时回退到手动重启弹窗。
    pub(crate) fn handle_device_lost(&mut self, ctx: &egui::Context) {
        if !self.autosave.lost_started {
            self.autosave.lost_started = true;
            self.begin_lost_recovery();
            return;
        }
        if let Some(rx) = self.autosave.lost_rx.as_ref() {
            match rx.try_recv() {
                Ok(true) => {
                    self.autosave.lost_rx = None;
                    match std::env::current_exe() {
                        Ok(exe) => {
                            let spawn = std::process::Command::new(exe)
                                .env(RECOVER_ENV, "1")
                                .spawn();
                            match spawn {
                                Ok(_) => {
                                    tracing::info!("[autosave] GPU 设备丢失：已启动恢复进程");
                                    self.should_exit = true;
                                }
                                Err(e) => {
                                    tracing::error!("[autosave] 恢复进程启动失败: {e}");
                                    self.autosave.lost_failed = true;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("[autosave] 无法定位可执行文件: {e}");
                            self.autosave.lost_failed = true;
                        }
                    }
                }
                Ok(false) => {
                    self.autosave.lost_rx = None;
                    self.autosave.lost_failed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.autosave.lost_rx = None;
                    self.autosave.lost_failed = true;
                }
            }
        }
        if self.autosave.lost_failed && crate::dialogs::gpu_device_lost::show_viewport(ctx) {
            self.should_exit = true;
        }
    }

    /// 保全所有未保存文档到 autosave 目录；未修改的文档记入 reopen。
    fn begin_lost_recovery(&mut self) {
        let mut jobs = Vec::new();
        let mut reopen = Vec::new();
        for idx in 0..self.workspace.documents.len() {
            let doc = &self.workspace.documents[idx];
            if doc.is_dirty() {
                jobs.push(idx);
            } else if let Some(p) = &doc.file_path {
                reopen.push(p.clone());
            }
        }
        if jobs.is_empty() && reopen.is_empty() {
            self.autosave.lost_failed = true;
            return;
        }
        if jobs.is_empty() {
            // 没有需要备份的文档：直接写 manifest 并让主循环 spawn
            save_manifest(&Manifest {
                auto_recover: true,
                entries: Vec::new(),
                reopen,
            });
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(true);
            self.autosave.lost_rx = Some(rx);
            return;
        }
        let jobs: Vec<AutoSaveJob> = jobs.into_iter().map(|idx| self.autosave_job(idx)).collect();
        self.spawn_autosave_thread(jobs, Some(LostContext { reopen }));
    }
}

struct AutoSaveJob {
    doc_id: u64,
    path: PathBuf,
    original: Option<String>,
    name: String,
    snapshot: SaveSnapshot,
}

/// 顺序保存一组任务，返回成功记录与是否全部成功。
fn save_jobs(jobs: Vec<AutoSaveJob>) -> (Vec<AutoSaveEntry>, bool) {
    let mut entries = Vec::new();
    let mut all_ok = true;
    for job in jobs {
        let path_str = job.path.to_string_lossy().to_string();
        let result = yinhe_yin::save_yin_with_files_progress(
            &job.snapshot.model,
            &path_str,
            &job.snapshot.project_file,
            &job.snapshot.mapping_file,
            Some(&job.snapshot.mixer),
            |_| {},
        );
        match result {
            Ok(()) => entries.push(AutoSaveEntry {
                doc_id: job.doc_id,
                file: path_str,
                original: job.original,
                name: job.name,
                saved_at: now_secs(),
                backup: true,
            }),
            Err(e) => {
                tracing::warn!("[autosave] 「{}」备份失败: {e}", job.name);
                all_ok = false;
            }
        }
    }
    (entries, all_ok)
}

struct LostContext {
    reopen: Vec<String>,
}
