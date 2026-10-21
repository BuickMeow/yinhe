//! 音频播放：CPAL 输出 + `yinhe-synth` CPU 合成（`SynthEngine::YinheCpu`）。
//!
//! - 音频惰性初始化：首次播放/加载文档时创建，失败只记日志（无音频设备时静默降级）。
//! - 音色库：`SetSoundFonts`（0..16 源通道共用同一库）。没有音色库时播放无声，
//!   但时钟/光标/调度照常工作（引擎语义如此）。
//! - 位置回读：`sample_position() / sample_rate` 秒数 → tick（tempo_map 二分）。
//! - 锁顺序约定：`scene` → `audio`（本模块内不反向嵌套）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use yinhe_audio::export::{ExportProgress, WavBitDepth};
use yinhe_audio::{AudioCommand, CpalAudioHandle, SynthEngine};
use yinhe_core::YinModel;
use yinhe_types::Interpolation;

use crate::scene;

struct AudioState {
    handle: CpalAudioHandle,
    /// 暂停/停止时的光标 tick（播放中由 sample_position 换算）。
    cursor_tick: f64,
    /// 最近一次已真正下发的音色库路径列表（避免重复解析）。
    sf_sent: Option<Vec<String>>,
}

/// 全局音色库条目：设置页可拖动排序 / 启用停用。
///
/// 顺序即优先级：列表靠前的库先被查询，命中即覆盖靠后的库
/// （xsynth `ChannelSoundfont` 按 `soundfonts` 顺序找第一个非空 spawner）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundfontEntry {
    pub path: String,
    pub enabled: bool,
}

/// 音频参数（设置页可调；max_voices=0 自动，buffer_size=0 设备默认）。
#[derive(Clone, Copy)]
pub struct AudioSettings {
    pub max_voices: u32,
    pub buffer_size: u32,
    pub ignore_velocity: u8,
}

/// “自动复音”的平台默认值。
///
/// 引擎的 `DEFAULT_MAX_VOICES=14336` 是桌面级上限；移动端黑乐谱峰值段
/// 即使 1024 也可能让合成线程超过 512 帧的实时预算，因此 Android 自动档
/// 收敛到 512，给调度留余量。用户仍可在设置里手动选 256/512/1024。
fn auto_max_voices() -> usize {
    if cfg!(target_os = "android") {
        512
    } else {
        yinhe_audio::DEFAULT_MAX_VOICES
    }
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            max_voices: 512,
            buffer_size: 2048,
            ignore_velocity: 1,
        }
    }
}

static AUDIO_SETTINGS: OnceLock<Mutex<AudioSettings>> = OnceLock::new();

fn settings_slot() -> &'static Mutex<AudioSettings> {
    AUDIO_SETTINGS.get_or_init(|| Mutex::new(AudioSettings::default()))
}

/// 后台 WAV 导出会话：渲染线程写 `path`，Flutter 轮询进度/取消。
struct ExportSession {
    progress: Arc<Mutex<ExportProgress>>,
    cancel: Arc<AtomicBool>,
}

static EXPORT_SESSION: OnceLock<Mutex<Option<ExportSession>>> = OnceLock::new();

fn export_slot() -> &'static Mutex<Option<ExportSession>> {
    EXPORT_SESSION.get_or_init(|| Mutex::new(None))
}

/// 开始导出 WAV；返回 1 = 已开始，0 = 无工程/音频不可用/正在导出。
pub fn start_export(path: &str, bit_depth: i32) -> i32 {
    if !ensure_audio() {
        return 0;
    }
    let Ok(mut slot) = export_slot().lock() else {
        return 0;
    };
    if slot
        .as_ref()
        .is_some_and(|s| s.progress.lock().map(|p| !p.finished).unwrap_or(false))
    {
        return 0;
    }
    let progress = ExportProgress::new();
    if let Ok(mut p) = progress.lock() {
        p.reset();
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let bit_depth = match bit_depth {
        24 => WavBitDepth::Bit24,
        32 => WavBitDepth::Bit32Float,
        _ => WavBitDepth::Bit16,
    };
    let Some(mut guard) = lock_audio() else {
        return 0;
    };
    let Some(st) = guard.as_mut() else {
        return 0;
    };
    // 采样率固定为当前设备采样率（插件按设备采样率激活）。
    st.handle.handle.send(AudioCommand::ExportStart {
        path: PathBuf::from(path),
        bit_depth,
        layer_count: None,
        restore_layer_count: None,
        progress: Arc::clone(&progress),
        cancel: Arc::clone(&cancel),
        pause: Arc::new(AtomicBool::new(false)),
    });
    *slot = Some(ExportSession { progress, cancel });
    1
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

/// 导出进度 JSON：state 0=无会话、1=导出中、2=完成、3=失败。
pub fn export_progress_json() -> String {
    let Ok(slot) = export_slot().lock() else {
        return "{\"state\":0}".into();
    };
    let Some(session) = slot.as_ref() else {
        return "{\"state\":0}".into();
    };
    let Ok(p) = session.progress.lock() else {
        return "{\"state\":0}".into();
    };
    let state = if p.finished {
        if p.error.is_some() { 3 } else { 2 }
    } else {
        1
    };
    format!(
        "{{\"state\":{},\"progress\":{:.4},\"status\":\"{}\",\"error\":\"{}\"}}",
        state,
        p.progress.clamp(0.0, 1.0),
        json_escape(&p.status),
        json_escape(p.error.as_deref().unwrap_or(""))
    )
}

pub fn cancel_export() {
    if let Ok(slot) = export_slot().lock()
        && let Some(session) = slot.as_ref()
    {
        session.cancel.store(true, Ordering::Relaxed);
    }
}

/// 当前音频设置。
pub fn settings() -> AudioSettings {
    settings_slot().lock().map(|g| *g).unwrap_or_default()
}

/// 应用音频设置：复音/忽略力度立即下发；buffer_size 下次音频初始化生效。
pub fn set_settings(max_voices: u32, buffer_size: u32, ignore_velocity: u8) {
    if let Ok(mut g) = settings_slot().lock() {
        *g = AudioSettings {
            max_voices,
            buffer_size,
            ignore_velocity,
        };
    }
    let Some(mut guard) = lock_audio() else {
        return;
    };
    let Some(st) = guard.as_mut() else {
        return;
    };
    apply_runtime_settings(st);
}

/// 音频设置 JSON（设置页回显）。
pub fn settings_json() -> String {
    let s = settings();
    format!(
        "{{\"max_voices\":{},\"buffer_size\":{},\"ignore_velocity\":{}}}",
        s.max_voices, s.buffer_size, s.ignore_velocity
    )
}

fn apply_runtime_settings(st: &AudioState) {
    let cfg = settings();
    st.handle.handle.send(AudioCommand::SetMaxVoices {
        max: Some(if cfg.max_voices == 0 {
            auto_max_voices()
        } else {
            cfg.max_voices as usize
        }),
    });
    st.handle.handle.send(AudioCommand::SetIgnoreVelocity {
        threshold: cfg.ignore_velocity,
    });
}

static AUDIO: OnceLock<Mutex<Option<AudioState>>> = OnceLock::new();
static SOUNDFONTS: OnceLock<Mutex<Vec<SoundfontEntry>>> = OnceLock::new();

fn state_slot() -> &'static Mutex<Option<AudioState>> {
    AUDIO.get_or_init(|| Mutex::new(None))
}

fn soundfonts_slot() -> &'static Mutex<Vec<SoundfontEntry>> {
    SOUNDFONTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// 当前全局音色库表（顺序 = 优先级）。
pub fn soundfonts() -> Vec<SoundfontEntry> {
    soundfonts_slot()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// 已启用音色库路径（按表顺序；未启用不参与播放）。
pub fn enabled_soundfonts() -> Vec<String> {
    soundfonts()
        .into_iter()
        .filter(|e| e.enabled)
        .map(|e| e.path)
        .collect()
}

/// 整体替换音色库表（设置页排序/启停/删除后调用），立即应用。
pub fn set_soundfonts(entries: Vec<SoundfontEntry>) {
    if let Ok(mut g) = soundfonts_slot().lock() {
        *g = entries;
    }
    let Some(mut guard) = lock_audio() else {
        return;
    };
    let Some(st) = guard.as_mut() else {
        return;
    };
    apply_soundfonts(st);
}

/// 追加一个音色库（已存在则启用并移到最前），立即应用。
pub fn add_soundfont(path: String) {
    if let Ok(mut g) = soundfonts_slot().lock() {
        match g.iter().position(|e| e.path == path) {
            Some(i) => {
                g[i].enabled = true;
                let e = g.remove(i);
                g.insert(0, e);
            }
            None => g.insert(
                0,
                SoundfontEntry {
                    path: path.clone(),
                    enabled: true,
                },
            ),
        }
    }
    let Some(mut guard) = lock_audio() else {
        return;
    };
    let Some(st) = guard.as_mut() else {
        return;
    };
    apply_soundfonts(st);
    log::info!("音色库已加入: {path}");
}

fn lock_audio() -> Option<std::sync::MutexGuard<'static, Option<AudioState>>> {
    match state_slot().lock() {
        Ok(g) => Some(g),
        Err(e) => Some(e.into_inner()),
    }
}

/// 首次使用时初始化音频；返回 false = 初始化失败。
fn ensure_audio() -> bool {
    let Some(mut guard) = lock_audio() else {
        return false;
    };
    if guard.is_some() {
        return true;
    }
    use yinhe_audio::channel_layout::ChannelLayout;
    let layout = ChannelLayout::from_mask(vec![true; 16]);
    let cfg = settings();
    // 缓冲默认 2048 帧：AAudio 动态调优在部分机型上收敛慢，固定值更稳
    // （cpal Fixed(n) 实际 capacity 为 2n；蓝牙 A2DP 路径需要更大缓冲）。
    let buffer = if cfg.buffer_size == 0 {
        cpal::BufferSize::Default
    } else {
        cpal::BufferSize::Fixed(cfg.buffer_size)
    };
    let spawned = yinhe_audio::spawn_cpal_audio(
        48_000,
        layout,
        buffer,
        None,
        SynthEngine::YinheCpu,
        Interpolation::default(),
    );
    match spawned {
        Ok(handle) => {
            log::info!("音频初始化完成: {} Hz", handle.sample_rate);
            let mut st = AudioState {
                handle,
                cursor_tick: 0.0,
                sf_sent: None,
            };
            apply_soundfonts(&mut st);
            // 应用设置页配置（默认 ignore_velocity=1、max_voices=512：
            // 1024 在 tau 峰值段仍可能超过 512 帧的实时预算）。
            apply_runtime_settings(&st);
            *guard = Some(st);
            true
        }
        Err(e) => {
            log::error!("音频初始化失败（播放不可用）: {e}");
            false
        }
    }
}

fn send_soundfonts(handle: &CpalAudioHandle, paths: &[String]) {
    // 0..16 = port0 的 16 个源通道共用同一份音色库表
    // （顺序 = 优先级，靠前者覆盖靠后者；多 port 后续扩展）。
    let configs: Vec<(u8, Vec<String>)> = (0u8..16).map(|ch| (ch, paths.to_vec())).collect();
    handle.handle.send(AudioCommand::SetSoundFonts {
        configs: Box::new(configs),
    });
}

/// 把当前启用的音色库表下发给引擎（与上次相同则跳过）。
fn apply_soundfonts(st: &mut AudioState) {
    let paths = enabled_soundfonts();
    if st.sf_sent.as_ref() == Some(&paths) {
        return;
    }
    send_soundfonts(&st.handle, &paths);
    st.sf_sent = Some(paths);
}

/// 轨道 mute/solo 掩码（与桌面 SkipTracks 同机制，超时/丢命令时保留最新值）。
pub fn set_skip_mask(skip: Vec<bool>) {
    let Some(mut guard) = lock_audio() else {
        return;
    };
    let Some(st) = guard.as_mut() else {
        return;
    };
    st.handle.handle.send(AudioCommand::SkipTracks { skip });
}

/// 音色库表 JSON（设置页用）：`[{"path":..,"name":..,"enabled":..}, ...]`。
pub fn soundfonts_json() -> String {
    let items: Vec<serde_json::Value> = soundfonts()
        .into_iter()
        .map(|e| {
            let name = std::path::Path::new(&e.path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            serde_json::json!({"path": e.path, "name": name, "enabled": e.enabled})
        })
        .collect();
    serde_json::Value::Array(items).to_string()
}

/// 解析并应用设置页传来的音色库表 JSON（顺序即优先级）。成功 1。
pub fn set_soundfonts_json(json: &str) -> i32 {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return 0;
    };
    let Some(items) = value.as_array() else {
        return 0;
    };
    let entries: Vec<SoundfontEntry> = items
        .iter()
        .filter_map(|it| {
            let path = it.get("path")?.as_str()?.to_string();
            let enabled = it
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            Some(SoundfontEntry { path, enabled })
        })
        .collect();
    set_soundfonts(entries);
    1
}

/// 音符编辑（移动/增删/改长度）：只重建 audible_notes，不中断播放。
pub fn update_notes(model: Arc<YinModel>) {
    let Some(mut guard) = lock_audio() else {
        return;
    };
    let Some(st) = guard.as_mut() else {
        return;
    };
    st.handle.handle.send(AudioCommand::UpdateNotes { model });
}

/// 自动化/轨道/撤销重做等结构性编辑：重建 CC + audible_notes，不中断播放。
pub fn reload_notes(model: Arc<YinModel>) {
    let Some(mut guard) = lock_audio() else {
        return;
    };
    let Some(st) = guard.as_mut() else {
        return;
    };
    st.handle.handle.send(AudioCommand::ReloadNotes { model });
}

/// 文档加载完成：把模型交给音频引擎，并确保音色库已下发。
pub fn on_model_loaded(model: Arc<YinModel>) {
    if !ensure_audio() {
        return;
    }
    let Some(mut guard) = lock_audio() else {
        return;
    };
    let Some(st) = guard.as_mut() else {
        return;
    };
    apply_soundfonts(st);
    st.handle.handle.send(AudioCommand::LoadModel { model });
    st.cursor_tick = 0.0;
    log::info!("音频模型已加载");
}

/// 播放（从暂停位置开始）。
pub fn play() {
    if !ensure_audio() {
        return;
    }
    let tick = {
        let Some(mut guard) = lock_audio() else {
            return;
        };
        let Some(st) = guard.as_mut() else {
            return;
        };
        flow_sync_cursor(st)
    };
    let seconds = model_seconds(tick);
    let Some(mut guard) = lock_audio() else {
        return;
    };
    if let Some(st) = guard.as_mut() {
        let from = (seconds * st.handle.sample_rate as f64) as u64;
        st.handle
            .handle
            .send(AudioCommand::Play { from_sample: from });
        log::info!("播放: tick={tick:.0} sample={from}");
    }
}

/// 暂停（记录当前光标位置）。
pub fn pause() {
    let tick = playback_tick();
    let Some(mut guard) = lock_audio() else {
        return;
    };
    if let Some(st) = guard.as_mut() {
        st.handle.handle.send(AudioCommand::Pause);
        st.cursor_tick = tick;
        log::info!("暂停: tick={tick:.0}");
    }
}

/// 停止（光标归零）。
pub fn stop() {
    let Some(mut guard) = lock_audio() else {
        return;
    };
    if let Some(st) = guard.as_mut() {
        st.handle.handle.send(AudioCommand::Stop);
        st.cursor_tick = 0.0;
        log::info!("停止");
    }
}

pub fn is_playing() -> bool {
    let Some(guard) = lock_audio() else {
        return false;
    };
    guard
        .as_ref()
        .is_some_and(|st| st.handle.handle.is_playing())
}

/// 跳转播放光标（tick）。播放中立即 seek 音频；停止时只移动光标。
pub fn seek(tick: f64) -> bool {
    let tick = tick.max(0.0);
    let seconds = model_seconds(tick);
    let Some(mut guard) = lock_audio() else {
        return false;
    };
    let Some(st) = guard.as_mut() else {
        return false;
    };
    st.cursor_tick = tick;
    if st.handle.handle.is_playing() {
        let sample = (seconds * st.handle.sample_rate as f64) as u64;
        st.handle.handle.send(AudioCommand::Seek { sample });
        log::info!("跳转播放位置: tick={tick:.0}");
    }
    true
}

/// 当前播放位置（tick）。播放中按采样位置换算，否则返回光标值。
pub fn playback_tick() -> f64 {
    let model = current_model();
    let Some(model) = model else {
        return 0.0;
    };
    playback_tick_for(&model)
}

/// 当前播放位置（tick），模型由调用方提供。
///
/// 供已持有 scene 锁的调用方使用（锁顺序 scene → audio，避免二次加锁死锁）。
pub fn playback_tick_for(model: &YinModel) -> f64 {
    let Some(mut guard) = lock_audio() else {
        return 0.0;
    };
    let Some(st) = guard.as_mut() else {
        return 0.0;
    };
    if st.handle.handle.is_playing() {
        let seconds = st.handle.handle.sample_position() as f64 / st.handle.sample_rate as f64;
        seconds_to_tick(model, seconds)
    } else {
        st.cursor_tick
    }
}

/// 当前播放位置（秒，供 UI 时间显示）。
pub fn playback_seconds() -> f64 {
    let model = current_model();
    let Some(model) = model else {
        return 0.0;
    };
    let tick = playback_tick();
    model.tempo_map.tick_to_seconds(tick as u64)
}

fn current_model() -> Option<Arc<YinModel>> {
    scene::scene().lock().ok().and_then(|s| s.model.clone())
}

fn model_seconds(tick: f64) -> f64 {
    current_model()
        .map(|m| m.tempo_map.tick_to_seconds(tick.max(0.0) as u64))
        .unwrap_or(0.0)
}

/// 播放中：用采样位置刷新光标；返回最新光标 tick。
fn flow_sync_cursor(st: &mut AudioState) -> f64 {
    if st.handle.handle.is_playing() {
        let model = current_model();
        if let Some(model) = model {
            let seconds = st.handle.handle.sample_position() as f64 / st.handle.sample_rate as f64;
            st.cursor_tick = seconds_to_tick(&model, seconds);
        }
    }
    st.cursor_tick
}

/// 秒数 → tick（`tick_to_seconds` 单调递增，二分 40 次）。
///
/// 上限用 `u32::MAX`（与桌面端一致）：光标超过末尾时不会被截断。
fn seconds_to_tick(model: &YinModel, seconds: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, u32::MAX as f64);
    for _ in 0..40 {
        let mid = (lo + hi) * 0.5;
        if model.tempo_map.tick_to_seconds(mid as u64) < seconds {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let tick = hi.round();
    // 0 BPM 剪切段内 tick_to_seconds 是常量：二分会落在剪切起点，
    // 这里直接推进到区间末端（跳转目标），否则播放头会停在剪切起点。
    for &(start, end) in &model.tempo_map.skip_ranges {
        if tick >= start as f64 && tick < end as f64 {
            return end as f64;
        }
    }
    tick
}
