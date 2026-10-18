//! 音频播放：CPAL 输出 + `yinhe-synth` CPU 合成（`SynthEngine::YinheCpu`）。
//!
//! - 音频惰性初始化：首次播放/加载文档时创建，失败只记日志（无音频设备时静默降级）。
//! - 音色库：`SetSoundFonts`（0..16 源通道共用同一库）。没有音色库时播放无声，
//!   但时钟/光标/调度照常工作（引擎语义如此）。
//! - 位置回读：`sample_position() / sample_rate` 秒数 → tick（tempo_map 二分）。
//! - 锁顺序约定：`scene` → `audio`（本模块内不反向嵌套）。

use std::sync::{Arc, Mutex, OnceLock};

use yinhe_audio::{AudioCommand, CpalAudioHandle, SynthEngine};
use yinhe_core::YinModel;
use yinhe_types::Interpolation;

use crate::scene;

struct AudioState {
    handle: CpalAudioHandle,
    /// 暂停/停止时的光标 tick（播放中由 sample_position 换算）。
    cursor_tick: f64,
    /// 当前音色库路径。
    soundfont: Option<String>,
    /// 最近一次已真正下发的音色库路径（避免重复解析）。
    sf_sent: Option<String>,
}

/// 音频参数（设置页可调；max_voices=0 自动，buffer_size=0 设备默认）。
#[derive(Clone, Copy)]
pub struct AudioSettings {
    pub max_voices: u32,
    pub buffer_size: u32,
    pub ignore_velocity: u8,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            max_voices: 1024,
            buffer_size: 2048,
            ignore_velocity: 1,
        }
    }
}

static AUDIO_SETTINGS: OnceLock<Mutex<AudioSettings>> = OnceLock::new();

fn settings_slot() -> &'static Mutex<AudioSettings> {
    AUDIO_SETTINGS.get_or_init(|| Mutex::new(AudioSettings::default()))
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
        max: if cfg.max_voices == 0 {
            None
        } else {
            Some(cfg.max_voices as usize)
        },
    });
    st.handle.handle.send(AudioCommand::SetIgnoreVelocity {
        threshold: cfg.ignore_velocity,
    });
}

static AUDIO: OnceLock<Mutex<Option<AudioState>>> = OnceLock::new();
static SOUNDFONT: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn state_slot() -> &'static Mutex<Option<AudioState>> {
    AUDIO.get_or_init(|| Mutex::new(None))
}

fn soundfont_slot() -> &'static Mutex<Option<String>> {
    SOUNDFONT.get_or_init(|| Mutex::new(None))
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
            let sf = soundfont_slot().lock().ok().and_then(|g| g.clone());
            let mut st = AudioState {
                handle,
                cursor_tick: 0.0,
                soundfont: sf.clone(),
                sf_sent: None,
            };
            if let Some(path) = &sf {
                send_soundfonts(&st.handle, path);
                st.sf_sent = Some(path.clone());
            }
            // 应用设置页配置（默认 ignore_velocity=1、max_voices=1024：
            // 桌面级复音 14336 在黑乐谱密集段会让合成线程超时）。
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

fn send_soundfonts(handle: &CpalAudioHandle, path: &str) {
    // 0..16 = port0 的 16 个源通道共用同一音色库（与旧壳一致；多 port 后续扩展）。
    let configs: Vec<(u8, Vec<String>)> =
        (0u8..16).map(|ch| (ch, vec![path.to_string()])).collect();
    handle.handle.send(AudioCommand::SetSoundFonts {
        configs: Box::new(configs),
    });
}

/// 当前音色库路径（可能尚未应用，仅记录）。无则空串。
pub fn soundfont_path() -> String {
    soundfont_slot()
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

/// 设置音色库路径（Kotlin 已复制到私有目录）。音频就绪时立即应用。
pub fn set_soundfont(path: String) {
    if let Ok(mut g) = soundfont_slot().lock() {
        *g = Some(path.clone());
    }
    let Some(mut guard) = lock_audio() else {
        return;
    };
    let Some(st) = guard.as_mut() else {
        // 音频尚未初始化：路径已记录，初始化时应用。
        return;
    };
    if st.sf_sent.as_deref() != Some(path.as_str()) {
        send_soundfonts(&st.handle, &path);
        st.sf_sent = Some(path.clone());
    }
    st.soundfont = Some(path.clone());
    log::info!("音色库已应用: {path}");
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

/// 文档加载完成：把模型交给音频引擎，并补发音色库（若已选）。
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
    if let Some(sf) = st.soundfont.clone()
        && st.sf_sent.as_deref() != Some(sf.as_str())
    {
        send_soundfonts(&st.handle, &sf);
        st.sf_sent = Some(sf.clone());
    }
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
    let Some(mut guard) = lock_audio() else {
        return 0.0;
    };
    let Some(st) = guard.as_mut() else {
        return 0.0;
    };
    if st.handle.handle.is_playing() {
        let seconds = st.handle.handle.sample_position() as f64 / st.handle.sample_rate as f64;
        seconds_to_tick(&model, seconds)
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
    hi.round()
}
