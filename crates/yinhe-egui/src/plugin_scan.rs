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
/// 单个 bundle 扫描子进程超时：挂死/无响应的插件此前会让启动页永不就绪。
const SCAN_JOB_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

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

/// 扫描缓存：每个 bundle 的文件指纹（mtime/size）+ 上次扫描的条目。
/// 指纹未变的 bundle 启动时直接复用（"加载过一次就不再重复加载"），
/// 只对新增/更新的 bundle 起子进程。
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct ScanCache {
    pub bundles: Vec<CachedBundle>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CachedBundle {
    pub path: String,
    pub mtime_secs: u64,
    pub size: u64,
    pub entries: Vec<PluginEntry>,
}

/// bundle 指纹（mtime 秒 + 文件大小）；读元数据失败返回 None（视为需重扫）。
fn bundle_fingerprint(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Some((mtime, meta.len()))
}

fn load_scan_cache() -> ScanCache {
    let path = yinhe_editor_core::paths::plugin_scan_cache_file();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_scan_cache(cache: &ScanCache) {
    let path = yinhe_editor_core::paths::plugin_scan_cache_file();
    match serde_json::to_string(cache) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!("写入插件扫描缓存失败: {e}");
            }
        }
        Err(e) => tracing::warn!("序列化插件扫描缓存失败: {e}"),
    }
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
            let cache = load_scan_cache();
            let (jobs, reused_entries, reused_bundles) = collect_jobs(&cache);
            // 缓存命中的条目先发（UI 立即有完整列表，只等新增/更新的 bundle）
            if !reused_entries.is_empty() {
                let _ = tx.send(ScanProgress::Batch(reused_entries));
            }
            let total = jobs.len();
            let errors = Arc::new(AtomicUsize::new(0));
            let queue = Arc::new(std::sync::Mutex::new(jobs));
            let scanned_bundles: Arc<std::sync::Mutex<Vec<CachedBundle>>> =
                Arc::new(std::sync::Mutex::new(reused_bundles));

            let worker_count = SCAN_WORKERS.min(total.max(1));
            let mut handles = Vec::new();
            for _ in 0..worker_count {
                let queue = Arc::clone(&queue);
                let tx = tx.clone();
                let errors = Arc::clone(&errors);
                let scanned_bundles = Arc::clone(&scanned_bundles);
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
                        } else {
                            // 只缓存成功扫描：失败的 bundle 每次重试（插件修复后
                            // 能自愈），不固化失败结果。
                            if let Some((mtime, size)) = bundle_fingerprint(job.path()) {
                                let bundle = CachedBundle {
                                    path: job.path().to_string_lossy().into_owned(),
                                    mtime_secs: mtime,
                                    size,
                                    entries: entries.clone(),
                                };
                                let mut all =
                                    scanned_bundles.lock().unwrap_or_else(|e| e.into_inner());
                                all.push(bundle);
                            }
                        }
                        let _ = tx.send(ScanProgress::Batch(entries));
                    }
                }));
            }
            for handle in handles {
                let _ = handle.join();
            }
            // 写回缓存（命中 + 新扫的成功项）
            {
                let all = scanned_bundles.lock().unwrap_or_else(|e| e.into_inner());
                save_scan_cache(&ScanCache {
                    bundles: all.clone(),
                });
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

/// 收集待扫描 bundle（纯文件系统）：缓存指纹（mtime/size）命中的 bundle
/// 直接复用条目、不重扫。返回 `(待扫任务, 复用的条目, 复用的缓存项)`。
fn collect_jobs(cache: &ScanCache) -> (Vec<ScanJob>, Vec<PluginEntry>, Vec<CachedBundle>) {
    let mut all: Vec<(PluginFormat, PathBuf)> = Vec::new();
    for path in yinhe_clap::scan::collect_bundles(&yinhe_clap::scan::default_plugin_dirs()) {
        all.push((PluginFormat::Clap, path));
    }
    for path in yinhe_vst3::scan::collect_bundle_paths(&yinhe_vst3::scan::default_plugin_dirs()) {
        all.push((PluginFormat::Vst3, path));
    }

    let mut jobs: Vec<ScanJob> = Vec::new();
    let mut reused_entries: Vec<PluginEntry> = Vec::new();
    let mut reused_bundles: Vec<CachedBundle> = Vec::new();
    for (format, path) in all {
        let path_str = path.to_string_lossy().into_owned();
        let hit = bundle_fingerprint(&path).and_then(|(mtime, size)| {
            cache
                .bundles
                .iter()
                .find(|b| b.path == path_str && b.mtime_secs == mtime && b.size == size)
        });
        match hit {
            Some(b) => {
                reused_entries.extend(b.entries.iter().cloned());
                reused_bundles.push(b.clone());
            }
            None => {
                let job = match format {
                    PluginFormat::Clap => ScanJob::Clap(path),
                    PluginFormat::Vst3 => ScanJob::Vst3(path),
                    PluginFormat::Builtin => continue,
                };
                jobs.push(job);
            }
        }
    }
    (jobs, reused_entries, reused_bundles)
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
    // spawn + 轮询（带超时）：插件挂死时 kill，绝不让扫描永久卡住。
    // stdout 用独立线程读取，避免插件大量噪声输出塞满管道导致死锁。
    let mut child = match std::process::Command::new(exe)
        .arg(SCAN_CHILD_ARG)
        .arg(job.format_arg())
        .arg(job.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                vec![PluginEntry::failed(
                    job.format(),
                    job.path(),
                    format!("启动扫描子进程失败: {e}"),
                )],
                1,
            );
        }
    };
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(s) = stdout.as_mut() {
            use std::io::Read;
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = std::time::Instant::now() + SCAN_JOB_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return (
                    vec![PluginEntry::failed(
                        job.format(),
                        job.path(),
                        format!("等待扫描子进程失败: {e}"),
                    )],
                    1,
                );
            }
        }
    };
    let bytes = reader.join().unwrap_or_default();
    let Some(status) = status else {
        return (
            vec![PluginEntry::failed(
                job.format(),
                job.path(),
                format!("扫描超时（>{:?}，插件可能挂死）", SCAN_JOB_TIMEOUT),
            )],
            1,
        );
    };
    if !status.success() {
        return (
            vec![PluginEntry::failed(
                job.format(),
                job.path(),
                format!(
                    "扫描子进程异常退出（{status}{}），插件可能已崩溃",
                    if status.code().is_none() {
                        " / 信号终止"
                    } else {
                        ""
                    }
                ),
            )],
            1,
        );
    }
    match parse_scan_output(&bytes) {
        Ok(result) => result,
        Err(e) => (vec![PluginEntry::failed(job.format(), job.path(), e)], 1),
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
