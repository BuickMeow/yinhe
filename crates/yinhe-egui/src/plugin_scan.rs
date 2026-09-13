//! 插件扫描：**逐 bundle 子进程隔离**。
//!
//! 背景：
//! - 部分插件（如 Maschine 3 的 NI/Qt）在加载时初始化 GUI 框架并要求调用线程
//!   是主线程；在后台线程加载会触发 libdispatch 断言直接杀进程。
//! - 单个损坏/崩溃的插件也会杀死整个扫描进程，导致全部结果丢失。
//!
//! 因此扫描拆成两段：
//! 1. 主进程纯文件系统收集 bundle 列表（安全、快速）；
//! 2. 每个 bundle 起一个子进程（Args: `--scan-plugins <clap|vst3> <path>`）加载，
//!    子进程在自己的主线程里加载插件；崩溃/失败只影响该 bundle，产出带
//!    `error` 的占位条目（UI 灰色显示，不静默消失）。
//!
//! 协议：子进程 stdout 输出 `BEGIN` / `<json>` / `END` 三行；
//! 插件自身的 stdout 噪声被标记解析忽略，stderr 丢弃。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;

use yinhe_mixer::PluginFormat;

use crate::mix::plugin_instance::PluginEntry;

/// 子进程扫描模式的命令行参数。
pub(crate) const SCAN_CHILD_ARG: &str = "--scan-plugins";
const BEGIN: &str = "###YINHE_SCAN_BEGIN###";
const END: &str = "###YINHE_SCAN_END###";
/// 并发扫描 worker 数（每个 worker 串行起子进程）。
const SCAN_WORKERS: usize = 4;

/// 一个待扫描的 bundle。
#[derive(Clone)]
enum ScanJob {
    Clap(PathBuf),
    Vst3(PathBuf),
}

impl ScanJob {
    fn format(&self) -> PluginFormat {
        match self {
            Self::Clap(_) => PluginFormat::Clap,
            Self::Vst3(_) => PluginFormat::Vst3,
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::Clap(p) | Self::Vst3(p) => p,
        }
    }

    fn format_arg(&self) -> &'static str {
        match self {
            Self::Clap(_) => "clap",
            Self::Vst3(_) => "vst3",
        }
    }
}

/// 扫描进度（流式；主进程 UI 逐步填充列表）。
pub(crate) enum ScanProgress {
    /// 一个 bundle 的扫描结果（失败时含带 `error` 的占位条目）。
    Batch(Vec<PluginEntry>),
    /// 全部完成。
    Finished { errors: usize },
}

/// 子进程入口：扫描单个 bundle 并输出 JSON（main 在 GUI 初始化前调用）。
pub(crate) fn run_scan_child(format: &str, path: Option<&Path>) {
    let (entries, errors): (Vec<PluginEntry>, usize) = match (format, path) {
        ("clap", Some(p)) => match yinhe_clap::scan::scan_bundle(p) {
            yinhe_clap::scan::ScanOutcome::Loaded(infos) => (
                infos
                    .into_iter()
                    .map(|i| {
                        let is_instrument = i.is_instrument();
                        let is_effect = i.is_audio_effect();
                        PluginEntry {
                            format: PluginFormat::Clap,
                            path: i.path,
                            id: i.id,
                            name: i.name,
                            vendor: i.vendor.unwrap_or_default(),
                            is_instrument,
                            is_effect,
                            error: None,
                        }
                    })
                    .collect(),
                0,
            ),
            yinhe_clap::scan::ScanOutcome::Failed { path, error } => (
                vec![PluginEntry::failed(
                    PluginFormat::Clap,
                    &path,
                    error.to_string(),
                )],
                1,
            ),
        },
        ("vst3", Some(p)) => match yinhe_vst3::scan::scan_bundle(p) {
            yinhe_vst3::scan::ScanOutcome::Loaded(infos) => (
                infos
                    .into_iter()
                    .map(|i| PluginEntry {
                        format: PluginFormat::Vst3,
                        path: i.path,
                        id: i.class_id,
                        name: i.name,
                        vendor: i.vendor,
                        is_instrument: i.is_instrument,
                        is_effect: i.is_effect,
                        error: None,
                    })
                    .collect(),
                0,
            ),
            yinhe_vst3::scan::ScanOutcome::Failed { path, error } => (
                vec![PluginEntry::failed(PluginFormat::Vst3, &path, error)],
                1,
            ),
        },
        _ => (Vec::new(), 1),
    };

    let payload = serde_json::json!({
        "plugins": entries,
        "errors": errors,
    });
    println!("{BEGIN}");
    println!("{payload}");
    println!("{END}");
}

/// 主进程：启动扫描 worker（流式进度），返回接收端。
pub(crate) fn spawn_scan_worker() -> Option<Receiver<ScanProgress>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawn_result = std::thread::Builder::new()
        .name("plugin-scan".into())
        .spawn(move || {
            let jobs: Vec<ScanJob> = collect_jobs();
            let total = jobs.len();
            let errors = Arc::new(AtomicUsize::new(0));
            let queue = Arc::new(std::sync::Mutex::new(jobs));

            let worker_count = SCAN_WORKERS.min(total.max(1));
            let mut handles = Vec::new();
            for _ in 0..worker_count {
                let queue = Arc::clone(&queue);
                let tx = tx.clone();
                let errors = Arc::clone(&errors);
                handles.push(std::thread::spawn(move || {
                    loop {
                        let job = {
                            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                            q.pop()
                        };
                        let Some(job) = job else { break };
                        let (entries, failed) = run_scan_job(&job);
                        if failed > 0 {
                            errors.fetch_add(1, Ordering::Relaxed);
                        }
                        let _ = tx.send(ScanProgress::Batch(entries));
                    }
                }));
            }
            for handle in handles {
                let _ = handle.join();
            }
            let _ = tx.send(ScanProgress::Finished {
                errors: errors.load(Ordering::Relaxed),
            });
        });
    match spawn_result {
        Ok(_) => Some(rx),
        Err(e) => {
            tracing::warn!("启动插件扫描线程失败: {e}");
            None
        }
    }
}

/// 收集全部待扫描 bundle（纯文件系统，不加载）。
fn collect_jobs() -> Vec<ScanJob> {
    let mut jobs: Vec<ScanJob> = Vec::new();
    for path in yinhe_clap::scan::collect_bundles(&yinhe_clap::scan::default_plugin_dirs()) {
        jobs.push(ScanJob::Clap(path));
    }
    for path in yinhe_vst3::scan::collect_bundle_paths(&yinhe_vst3::scan::default_plugin_dirs()) {
        jobs.push(ScanJob::Vst3(path));
    }
    jobs
}

/// 跑一个 bundle 的扫描子进程；返回 `(条目, 失败数)`。
fn run_scan_job(job: &ScanJob) -> (Vec<PluginEntry>, usize) {
    let Ok(exe) = std::env::current_exe() else {
        return (
            vec![PluginEntry::failed(
                job.format(),
                job.path(),
                "无法定位自身可执行文件".into(),
            )],
            1,
        );
    };
    let output = std::process::Command::new(exe)
        .arg(SCAN_CHILD_ARG)
        .arg(job.format_arg())
        .arg(job.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    match output {
        Ok(out) if out.status.success() => match parse_scan_output(&out.stdout) {
            Ok(result) => result,
            Err(e) => (vec![PluginEntry::failed(job.format(), job.path(), e)], 1),
        },
        Ok(out) => (
            vec![PluginEntry::failed(
                job.format(),
                job.path(),
                format!(
                    "扫描子进程异常退出（{}{}），插件可能已崩溃",
                    out.status,
                    if out.status.code().is_none() {
                        " / 信号终止"
                    } else {
                        ""
                    }
                ),
            )],
            1,
        ),
        Err(e) => (
            vec![PluginEntry::failed(
                job.format(),
                job.path(),
                format!("启动扫描子进程失败: {e}"),
            )],
            1,
        ),
    }
}

/// 解析子进程输出（标记之间的 JSON）。
fn parse_scan_output(bytes: &[u8]) -> Result<(Vec<PluginEntry>, usize), String> {
    let text = String::from_utf8_lossy(bytes);
    let begin = text.find(BEGIN).ok_or("扫描输出缺少开始标记")?;
    let end = text.find(END).ok_or("扫描输出缺少结束标记")?;
    if end <= begin {
        return Err("扫描输出标记顺序异常".into());
    }
    let json = text[begin + BEGIN.len()..end].trim();
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("解析扫描结果失败: {e}"))?;
    let plugins: Vec<PluginEntry> = serde_json::from_value(
        value
            .get("plugins")
            .cloned()
            .ok_or("扫描结果缺少 plugins 字段")?,
    )
    .map_err(|e| format!("解析插件列表失败: {e}"))?;
    let errors = value.get("errors").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    Ok((plugins, errors))
}
