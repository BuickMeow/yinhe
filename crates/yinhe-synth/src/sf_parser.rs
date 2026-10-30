//! SoundFont 解析器 — 统一支持 SFZ 和 SF2 格式。
//!
//! 委托 xsynth-soundfonts 解析 SFZ/SF2，按 (key, vel) 展开为**最终合成参数快照**，
//! 公式与 xsynth `SampleSoundfont` 的 spawner 构建逻辑完全对齐
//! （音量曲线、声像、滤波器、包络、vel2release 修正全部在 build 时算好）。
//! `note_on` 时零公式计算，直接消费快照字段。
//!
//! 采样数据统一重采样到目标采样率后以 `Arc<[f32]>` 共享（SFZ/SF2 同一路径），
//! offset/loop 索引同步换算到目标采样率，消除双重重采样。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use xsynth_soundfonts::FilterType;

/// Butterworth Q 值（与 xsynth 一致，来自 biquad crate）。
const Q_BUTTERWORTH: f32 = std::f32::consts::FRAC_1_SQRT_2;

mod sf2;
mod sfz;

/// 滤波器类型 → 渲染器编码（WGSL/CPU voice 共用；shader `filter_type` 字段）。
pub(crate) fn filter_type_code(ft: FilterType) -> u32 {
    match ft {
        FilterType::LowPass => 0,
        FilterType::HighPass => 1,
        FilterType::BandPass => 2,
        FilterType::LowPassPole => 3,
    }
}

/// 每个 (MIDI key, velocity) 对应的最终合成参数快照。
///
/// 展开语义与 xsynth `SampleVoiceSpawnerParams` 对齐：
/// - SFZ 的 `amp_veltrack` 二次曲线、`amp_keytrack`、`pan_veltrack/keytrack`、
///   `fil_veltrack/keytrack`、`ampeg_vel2release` 均已折算进各字段
/// - SF2 的 `note_params`（modulator 系统）在 build 时按 (key, vel) 展开
#[derive(Clone, Debug)]
pub struct KeyInfo {
    /// 重采样到目标采样率后的采样数据（Arc 共享，clone 零拷贝）。
    /// 立体声样本为 LRLR 交错存储，`is_stereo` 标记布局。
    pub sample_data: Arc<[f32]>,
    /// 采样是否为交错立体声（false = 单声道）。
    pub is_stereo: bool,
    /// 插值器：0=Nearest, 1=Linear（1 字节足够，避免 4 字节对齐膨胀）。
    pub interp: u8,

    /// 采样播放倍率（键位频率比 × 调音音分，等价 xsynth `get_speed_mult_from_keys` × `cents_factor`）。
    pub speed_mult: f32,
    /// 线性增益（含 vel 曲线与键位音量跟踪，等价 xsynth spawner 的 `volume`）。
    pub volume: f32,
    /// 采样起始偏移（帧，已按目标采样率换算）。
    pub offset: u32,
    /// 采样结束位置（帧，已换算；SF2 的 sample_end，等价 xsynth LoopParams.stop）。
    /// `u32::MAX` 表示无（SFZ 无此概念，播放到采样末尾）。
    /// 播放长度 = min(采样长度, stop) - offset。
    pub stop: u32,

    // ── ADSR 包络（秒；攻击/释放已有 0.001s 下限防除零）──
    pub ampeg_start: f32, // 0..1
    pub ampeg_delay: f32,
    pub ampeg_attack: f32,
    pub ampeg_hold: f32,
    pub ampeg_decay: f32,
    pub ampeg_sustain: f32, // 0..1
    pub ampeg_release: f32, // 已含 ampeg_vel2release 修正

    // ── 力度分层（展开后为精确单值）──
    pub lovel: u8,
    pub hivel: u8,

    // ── 循环 ──
    pub loop_mode: LoopMode,
    pub loop_start: u32, // 帧，已按目标采样率换算
    pub loop_end: u32,

    // ── 滤波器（cutoff=0 表示无滤波器，与 xsynth `use_effects` 一致）──
    pub cutoff: f32,    // Hz，已含 fil_veltrack/keytrack 修正并 clamp
    pub resonance: f32, // 线性（db_to_amp(dB) × Q_BUTTERWORTH）
    pub filter_type: FilterType,

    // ── 加载期烘焙（运行期每 note_on 零三角函数）──
    /// per-voice biquad 系数 `[b0,b1,b2,a1,a2]`（cutoff<=0 时 None）。
    pub biquad: Option<[f32; 5]>,
    /// 等功率声像增益（`pan` 的 cos/sin 预计算，与 CpuVoice::new 原公式一致）。
    pub pan_l: f32,
    pub pan_r: f32,
}

/// 加载期烘焙：等功率声像增益（`CpuVoice::new` 原公式的预计算）。
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = pan * std::f32::consts::FRAC_PI_2;
    ((angle.cos() * 1.42).min(1.0), (angle.sin() * 1.42).min(1.0))
}

/// 加载期烘焙：biquad 系数（cutoff<=0 → None，与 `CpuVoice::new` 一致）。
fn bake_biquad(
    cutoff: f32,
    resonance: f32,
    filter_type: FilterType,
    sample_rate: u32,
) -> Option<[f32; 5]> {
    (cutoff > 0.0).then(|| {
        let (b0, b1, b2, a1, a2) = crate::synth::biquad_coeffs(
            filter_type_code(filter_type),
            cutoff,
            resonance,
            sample_rate as f32,
        );
        [b0, b1, b2, a1, a2]
    })
}

/// 采样循环模式
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopMode {
    NoLoop,
    LoopContinuous,
    LoopSustain,
    OneShot,
}

impl Default for KeyInfo {
    fn default() -> Self {
        Self {
            sample_data: Arc::from([]),
            is_stereo: false,
            interp: 0,
            speed_mult: 1.0,
            volume: 1.0,
            offset: 0,
            stop: u32::MAX,
            ampeg_start: 0.0,
            ampeg_delay: 0.0,
            // 默认对齐 xsynth（`AmpegEnvelopeParams::default`）：attack/release
            // 10ms 而非 SFZ 规范的 1ms——音色库不写 ampeg_release 时（如 Starry
            // Studio Grand），1ms 包络让每个音符结束都像硬切（密集音符连成
            // click），xsynth 的 10ms 听感自然。
            ampeg_attack: 0.01,
            ampeg_hold: 0.0,
            ampeg_decay: 0.001,
            ampeg_sustain: 1.0,
            ampeg_release: 0.01,
            lovel: 0,
            hivel: 127,
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: 0,
            cutoff: 0.0,
            resonance: Q_BUTTERWORTH,
            filter_type: FilterType::default(),
            biquad: None,
            pan_l: 1.0,
            pan_r: 1.0,
        }
    }
}

/// 一个音色库文件中的一个（bank, preset）条目及其 key map。
///
/// 语义与 xsynth `SoundfontInstrument` 对齐：SFZ 无 preset 概念，整个文件是
/// (0, 0) 一个条目（等价 xsynth `SoundfontInitOptions` 默认）；SF2 每个 preset
/// 一个条目。program（bank, preset）选择条目，选不到则静音。
///
/// key map 采用**惰性展开**（SF2）：只有 program 命中该 (bank, preset)、
/// 查询到该条目时才展开。GM 库常含数百个 preset，全量展开会生成巨量 KeyInfo
/// （GeneralUser 287 preset ≈ 670 万条 / ~800MB），而工程通常只用其中几个。
#[derive(Clone)]
pub struct KeyMapEntry {
    pub bank: u8,
    pub preset: u8,
    inner: Arc<KeyMapEntryInner>,
}

struct KeyMapEntryInner {
    /// 展开结果（SFZ 构造时即填好；SF2 首次查询时惰性填充）。
    map: OnceLock<Vec<Vec<KeyInfo>>>,
    /// 惰性展开源（SF2 多 preset 用；SFZ 为 None）。
    expand: Option<Box<dyn Fn() -> Vec<Vec<KeyInfo>> + Send + Sync>>,
}

impl KeyMapEntry {
    /// 预展开条目（SFZ：整个文件一个条目，无惰性收益）。
    pub(crate) fn expanded(bank: u8, preset: u8, mut map: Vec<Vec<KeyInfo>>) -> Self {
        merge_stereo_pairs(&mut map);
        let map_cell = OnceLock::new();
        let _ = map_cell.set(map);
        Self {
            bank,
            preset,
            inner: Arc::new(KeyMapEntryInner {
                map: map_cell,
                expand: None,
            }),
        }
    }

    /// 惰性条目（SF2：首次查询命中该 preset 时才调用 `expand`）。
    pub(crate) fn lazy(
        bank: u8,
        preset: u8,
        expand: impl Fn() -> Vec<Vec<KeyInfo>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            bank,
            preset,
            inner: Arc::new(KeyMapEntryInner {
                map: OnceLock::new(),
                expand: Some(Box::new(expand)),
            }),
        }
    }

    /// (key, vel) 展开后的参数快照；首次调用触发惰性展开。
    pub fn map(&self) -> &Vec<Vec<KeyInfo>> {
        self.inner.map.get_or_init(|| {
            let mut map = match &self.inner.expand {
                Some(f) => f(),
                None => Vec::new(),
            };
            merge_stereo_pairs(&mut map);
            map
        })
    }

    /// 是否被缓存之外的引用持有。
    ///
    /// 多库合并（`load_key_maps_merged`）会把各库条目**克隆**进合并 Vec，克隆
    /// 共享同一 `inner`。此时该库的缓存 `Arc<Vec<KeyMapEntry>>` 引用计数仍是 1
    /// （合并 Vec 不持有它），但 `inner` 计数 > 1。`sweep_key_map_cache` 必须据此
    /// 判断"仍在使用"，否则会误删正在用的库，导致每次加载都重新解析、内存累积。
    pub(crate) fn is_shared(&self) -> bool {
        Arc::strong_count(&self.inner) > 1
    }
}

impl std::fmt::Debug for KeyMapEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyMapEntry")
            .field("bank", &self.bank)
            .field("preset", &self.preset)
            .field("expanded", &self.inner.map.get().is_some())
            .finish()
    }
}

/// 根据文件扩展名自动检测格式并构建 key map 条目列表。
/// `sample_rate` 为目标采样率：SFZ 的 wav 与 SF2 都在此采样率下加载/重采样一次。
pub fn build_key_maps(
    path: &Path,
    sample_rate: u32,
    interp: u32,
) -> Result<Vec<KeyMapEntry>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    let mut entries = match ext.as_deref() {
        Some("sfz") => vec![KeyMapEntry::expanded(
            0,
            0,
            sfz::build_key_map_from_sfz(path, sample_rate, interp)?,
        )],
        Some("sf2") => sf2::build_key_maps_from_sf2(path, sample_rate, interp)?,
        _ => return Err(format!("Unsupported soundfont format: {:?}", path)),
    };
    // 按 (bank, preset) 升序，使 `select_key_info_multi` 能二分定位（GM 库常含
    // 数百 preset，逐个线性扫描是 note_on 热路径的开销）。稳定排序保留同键条目
    // 的原始先后，"区间内首个命中条目胜出"的语义不变。
    entries.sort_by_key(|e| (e.bank, e.preset));
    Ok(entries)
}

/// 多音色库 + program 选择（与 xsynth `ChannelSoundfont::rebuild_matrix` 一致）：
/// 1. 主选：按文件顺序找 (bank, preset) 匹配且该 (key, vel) 有 region 的条目；
/// 2. 兜底：全部主选落空后，鼓组（bank==128）找 (128, 0)，旋律找 (0, preset)；
/// 3. 都落空 → 静音（与 xsynth 缺失 preset 静音一致）。
///
/// 返回该 (key, vel) 的**全部** region 层：xsynth 一个 NoteOn 会为每个 spawner
/// 创建一个 voice，多层叠加是音色本身的一部分（鼓的 L/R 对、叠加强度层）。
/// 调用方必须为每层创建一个 voice，并用同一组号绑定它们的生命周期。
///
/// `entries` 必须按 `(bank, preset)` 升序（`build_key_maps`/`load_key_maps_merged`
/// 已保证），据此二分定位 program 区间。
pub fn select_key_infos_multi(
    entries: &[KeyMapEntry],
    bank: u8,
    preset: u8,
    key: u8,
    velocity: u8,
) -> Option<&[KeyInfo]> {
    if let Some(infos) = select_in_program(entries, bank, preset, key, velocity) {
        return Some(infos);
    }
    let (rb, rp) = if bank == 128 { (128, 0) } else { (0, preset) };
    if let Some(infos) = select_in_program(entries, rb, rp, key, velocity) {
        return Some(infos);
    }
    // 整个音色库没有鼓 bank（典型：SFZ 单乐器库只有一个 (0, 0) 条目）：
    // 通道 10 被置成 bank 128 后不该直接静音，回退到普通 bank 的同 program。
    // GM SF2 有 bank 128，上面的查询已经命中，不会走到这里。
    if bank == 128 && !entries.iter().any(|e| e.bank == 128) {
        return select_in_program(entries, 0, preset, key, velocity);
    }
    None
}

/// 二分定位 `(bank, preset)` 条目区间，返回区间内首个对该 `(key, vel)` 有 region
/// 的条目的层列表（保持原"首个命中条目胜出"语义；条目内所有层都返回）。
fn select_in_program(
    entries: &[KeyMapEntry],
    bank: u8,
    preset: u8,
    key: u8,
    velocity: u8,
) -> Option<&[KeyInfo]> {
    let lo = entries.partition_point(|e| (e.bank, e.preset) < (bank, preset));
    for e in &entries[lo..] {
        if e.bank != bank || e.preset != preset {
            break;
        }
        if let Some(infos) = select_key_infos(e.map(), key, velocity) {
            return Some(infos);
        }
    }
    None
}

/// `select_key_infos_multi` 的单层便捷版（诊断/测试用：取命中层的第一层）。
pub fn select_key_info_multi(
    entries: &[KeyMapEntry],
    bank: u8,
    preset: u8,
    key: u8,
    velocity: u8,
) -> Option<&KeyInfo> {
    select_key_infos_multi(entries, bank, preset, key, velocity).and_then(|s| s.first())
}

/// 根据 key 和 velocity 选择对应的全部 KeyInfo 层（力度分层）。
///
/// 展开后每个 (key, vel) 的层按 `lovel` 排序且相邻，同一 vel 的叠加层是连续
/// 区间；正常情况精确命中，兜底选距离最近的 vel 的全部层。
///
/// key ≥ 128（256 键扩展音域）时音色库无对应区域：返回 None（静音），不越界。
pub fn select_key_infos(key_map: &[Vec<KeyInfo>], key: u8, velocity: u8) -> Option<&[KeyInfo]> {
    let layers = key_map.get(key as usize)?;
    if layers.is_empty() {
        return None;
    }
    if let Some(start) = layers
        .iter()
        .position(|info| velocity >= info.lovel && velocity <= info.hivel)
    {
        let (lovel, hivel) = (layers[start].lovel, layers[start].hivel);
        let end = start
            + layers[start..]
                .iter()
                .take_while(|info| info.lovel == lovel && info.hivel == hivel)
                .count();
        return Some(&layers[start..end]);
    }
    // 兜底：最近 vel 区间，返回该区间内的全部层（而不是只取一层）。
    let (lovel, hivel) = layers
        .iter()
        .min_by_key(|info| {
            (velocity as i16 - info.lovel as i16)
                .unsigned_abs()
                .min((velocity as i16 - info.hivel as i16).unsigned_abs())
        })
        .map(|info| (info.lovel, info.hivel))?;
    let start = layers
        .iter()
        .position(|info| info.lovel == lovel && info.hivel == hivel)?;
    let end = start
        + layers[start..]
            .iter()
            .take_while(|info| info.lovel == lovel && info.hivel == hivel)
            .count();
    Some(&layers[start..end])
}

/// `select_key_infos` 的单层便捷版（诊断/测试用：取命中层的第一层）。
pub fn select_key_info(key_map: &[Vec<KeyInfo>], key: u8, velocity: u8) -> Option<&KeyInfo> {
    select_key_infos(key_map, key, velocity).and_then(|s| s.first())
}

/// 把同一 (key, vel) 的左/右层合并成一个立体声层。
///
/// 两种来源：
/// - Polyphone 从 SF2 导出的 SFZ 常用两个 mono region 拼左右声道
///   （`sample=...L.wav pan=-100` + `...R.wav pan=+100`）；
/// - SF2 立体声 region 被 xsynth-soundfonts 拆成同一样本的两个 pan 极值 zone
///   （两层引用同一份交错立体声 sample）。
///
/// 每个 (key, vel) 只播一层时另一种声道会精确为 0（表现为“只响一边”，
/// THDrum 这类库整鼓偏左）。这里在加载期合并，播放侧无需感知，也不增加 voice。
fn merge_stereo_pairs(map: &mut [Vec<KeyInfo>]) {
    for layers in map {
        let mut i = 0;
        while i < layers.len() {
            if !is_hard_left(&layers[i]) {
                i += 1;
                continue;
            }
            let Some(j) = (i + 1..layers.len())
                .find(|&j| is_hard_right(&layers[j]) && same_layer_params(&layers[i], &layers[j]))
            else {
                i += 1;
                continue;
            };
            if layers[i].is_stereo
                && layers[j].is_stereo
                && std::ptr::eq(
                    layers[i].sample_data.as_ptr(),
                    layers[j].sample_data.as_ptr(),
                )
            {
                // 同一份交错立体声采样的 L/R pan 拆分：原样播一次即可，
                // 不能按 mono 重组（否则会把 L/R 交叉读错）。
                layers.remove(j);
                layers[i].pan_l = 1.0;
                layers[i].pan_r = 1.0;
                i += 1;
                continue;
            }
            if layers[i].is_stereo || layers[j].is_stereo {
                // 混合布局（一份立体声 + 一份 mono）没有安全的合并语义，保留两层。
                i += 1;
                continue;
            }
            // 先取出两份 mono 数据，再移除右侧层，最后写回合并结果。
            let left = layers[i].sample_data.clone();
            let right = layers[j].sample_data.clone();
            layers.remove(j);
            let frames = left.len().max(right.len());
            let mut merged = Vec::with_capacity(frames * 2);
            for k in 0..frames {
                merged.push(left.get(k).copied().unwrap_or(0.0));
                merged.push(right.get(k).copied().unwrap_or(0.0));
            }
            let layer = &mut layers[i];
            layer.sample_data = Arc::from(merged);
            layer.is_stereo = true;
            // 采样本身已经是 L/R，不能再叠加声像衰减。
            layer.pan_l = 1.0;
            layer.pan_r = 1.0;
            i += 1;
        }
    }
}

fn is_hard_left(info: &KeyInfo) -> bool {
    info.pan_l >= 0.999 && info.pan_r <= 0.001
}

fn is_hard_right(info: &KeyInfo) -> bool {
    info.pan_l <= 0.001 && info.pan_r >= 0.999
}

/// 合并前确认两条 layer 除左右声道外参数完全一致（音量/包络/循环/滤波等）。
fn same_layer_params(a: &KeyInfo, b: &KeyInfo) -> bool {
    const EPS: f32 = 1e-5;
    a.lovel == b.lovel
        && a.hivel == b.hivel
        && a.interp == b.interp
        && a.offset == b.offset
        && a.stop == b.stop
        && a.loop_mode == b.loop_mode
        && a.loop_start == b.loop_start
        && a.loop_end == b.loop_end
        && filter_type_code(a.filter_type) == filter_type_code(b.filter_type)
        && a.biquad == b.biquad
        && (a.speed_mult - b.speed_mult).abs() <= EPS
        && (a.volume - b.volume).abs() <= EPS
        && (a.ampeg_start - b.ampeg_start).abs() <= EPS
        && (a.ampeg_delay - b.ampeg_delay).abs() <= EPS
        && (a.ampeg_attack - b.ampeg_attack).abs() <= EPS
        && (a.ampeg_hold - b.ampeg_hold).abs() <= EPS
        && (a.ampeg_decay - b.ampeg_decay).abs() <= EPS
        && (a.ampeg_sustain - b.ampeg_sustain).abs() <= EPS
        && (a.ampeg_release - b.ampeg_release).abs() <= EPS
        && (a.cutoff - b.cutoff).abs() <= EPS
        && (a.resonance - b.resonance).abs() <= EPS
}

// ── SFZ ──

// ── 工具函数 ──

fn convert_loop_mode(mode: xsynth_soundfonts::LoopMode) -> LoopMode {
    match mode {
        xsynth_soundfonts::LoopMode::NoLoop => LoopMode::NoLoop,
        xsynth_soundfonts::LoopMode::LoopContinuous => LoopMode::LoopContinuous,
        xsynth_soundfonts::LoopMode::LoopSustain => LoopMode::LoopSustain,
        xsynth_soundfonts::LoopMode::OneShot => LoopMode::OneShot,
    }
}

/// 对交错立体声数据重采样：先按声道解交错，逐声道重采样后再交错。
fn resample_interleaved(raw: Vec<f32>, src_sr: u32, dst_sr: u32) -> Arc<[f32]> {
    let left: Vec<f32> = raw.iter().step_by(2).copied().collect();
    let right: Vec<f32> = raw.iter().skip(1).step_by(2).copied().collect();
    let left = xsynth_soundfonts::resample::resample_vec(left, src_sr as f32, dst_sr as f32);
    let right = xsynth_soundfonts::resample::resample_vec(right, src_sr as f32, dst_sr as f32);
    let len = left.len().min(right.len());
    let mut out = Vec::with_capacity(len * 2);
    for i in 0..len {
        out.push(left[i]);
        out.push(right[i]);
    }
    Arc::from(out)
}

/// Load a WAV file as f32 samples.
/// 立体声返回 LRLR 交错数据（不再平均成单声道），单声道返回原样。
/// 返回 (samples, sample_rate, is_stereo)。
pub fn load_wav_as_f32(path: &Path) -> Result<(Vec<f32>, u32, bool), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("Failed to open WAV {:?}: {}", path, e))?;

    let spec = reader.spec();
    // 读取样本必须显式处理 Err：损坏/截断的 WAV 会令 hound 返回错误，
    // 若 unwrap 则 GPU 加载音色库时直接 panic（release=abort）闪退。
    let samples: Vec<f32> = match spec.bits_per_sample {
        16 => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read 16-bit samples from {:?}: {}", path, e))?,
        24 => reader
            .samples::<i32>()
            .map(|s| s.map(|v| (v >> 8) as f32 / (i16::MAX as f32)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read 24-bit samples from {:?}: {}", path, e))?,
        32 => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / i32::MAX as f32))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read 32-bit samples from {:?}: {}", path, e))?,
        _ => return Err(format!("Unsupported bit depth: {}", spec.bits_per_sample)),
    };

    // 声道数：1=单声道，2=立体声（已是 LRLR 交错），>2 取前两声道
    match spec.channels {
        1 => Ok((samples, spec.sample_rate, false)),
        2 => Ok((samples, spec.sample_rate, true)),
        n => {
            let stereo: Vec<f32> = samples
                .chunks(n as usize)
                .flat_map(|ch| [ch[0], ch[1]])
                .collect();
            Ok((stereo, spec.sample_rate, true))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：单声道 wav 且源采样率 != 目标采样率时，重采样必须按单声道序列
    /// 处理（曾走交错路径，样本被按奇偶拆成两个"半速声道"，波形错乱）。
    #[test]
    fn mono_wav_resampled_as_mono() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wav_path = dir.path().join("tone.wav");
        let sfz_path = dir.path().join("tone.sfz");

        let src_sr = 44_100u32;
        let len = 4096usize;
        let samples: Vec<f32> = (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / src_sr as f32).sin() * 0.5)
            .collect();
        let mut writer = hound::WavWriter::create(
            &wav_path,
            hound::WavSpec {
                channels: 1,
                sample_rate: src_sr,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .expect("wav create");
        for &s in &samples {
            writer
                .write_sample((s * i16::MAX as f32) as i16)
                .expect("wav write");
        }
        writer.finalize().expect("wav finalize");
        std::fs::write(&sfz_path, "<region>\nsample=tone.wav key=60\n").expect("sfz write");

        let dst_sr = 48_000u32;
        let entries = build_key_maps(&sfz_path, dst_sr, 0).expect("build key maps");
        let info = &entries[0].map()[60][0];
        assert!(!info.is_stereo, "单声道样本必须标记为非立体声");

        // 与单声道重采样参考逐样本一致（交错路径的输出会明显不同）
        let (raw, _, is_stereo) = load_wav_as_f32(&wav_path).expect("load wav");
        assert!(!is_stereo);
        let expected = xsynth_soundfonts::resample::resample_vec(raw, src_sr as f32, dst_sr as f32);
        assert_eq!(info.sample_data.len(), expected.len());
        for (i, (a, b)) in info.sample_data.iter().zip(expected.iter()).enumerate() {
            assert!((a - b).abs() < 1e-6, "sample {i}: {a} vs {b}");
        }
    }

    fn entry(bank: u8, preset: u8, key: u8, vol: f32) -> KeyMapEntry {
        let mut map = vec![Vec::new(); 128];
        map[key as usize] = vec![KeyInfo {
            volume: vol,
            ..Default::default()
        }];
        KeyMapEntry::expanded(bank, preset, map)
    }

    /// G 回归：`select_key_info_multi` 改为按 (bank, preset) 二分后，命中、
    /// 同键条目区间遍历（首个胜出）、鼓组/旋律兜底都必须与旧线性扫描一致。
    #[test]
    fn select_key_info_multi_binary_search_matches_linear_semantics() {
        let mut entries = vec![
            entry(2, 10, 60, 1.0),
            entry(0, 5, 60, 1.0),
            entry(0, 5, 60, 2.0), // 同 (0,5) 同 key：稳定排序后应仍首个胜出
            entry(0, 5, 64, 3.0), // 同 (0,5) 另一 key：区间内需继续遍历
            entry(128, 0, 38, 4.0),
        ];
        entries.sort_by_key(|e| (e.bank, e.preset));

        let hit = select_key_info_multi(&entries, 0, 5, 60, 100).expect("(0,5) key60");
        assert_eq!(hit.volume, 1.0, "同 (bank,preset) 同 key 应首个命中胜出");
        assert!(select_key_info_multi(&entries, 0, 5, 64, 100).is_some());
        assert!(
            select_key_info_multi(&entries, 0, 9, 60, 100).is_none(),
            "旋律兜底 (0,9) 不存在 → 静音"
        );
        let drum = select_key_info_multi(&entries, 128, 9, 38, 100).expect("鼓组兜底 (128,0)");
        assert_eq!(drum.volume, 4.0);
        assert!(select_key_info_multi(&entries, 128, 9, 60, 100).is_none());
    }

    /// 回归：Polyphone 导出的 SFZ 用 L/R 两个 mono region 拼立体声；
    /// 加载期应合并成一个交错立体声层，否则每个 (key,vel) 只播一边。
    #[test]
    fn merges_hard_left_right_layers_into_stereo() {
        let mut map = vec![Vec::new(); 128];
        let left = KeyInfo {
            lovel: 60,
            hivel: 60,
            pan_l: 1.0,
            pan_r: 0.0,
            sample_data: Arc::from(vec![0.5f32, 0.6]),
            ..Default::default()
        };
        let right = KeyInfo {
            lovel: 60,
            hivel: 60,
            pan_l: 0.0,
            pan_r: 1.0,
            sample_data: Arc::from(vec![0.1f32, 0.2]),
            ..Default::default()
        };
        map[60] = vec![left, right];
        merge_stereo_pairs(&mut map);
        assert_eq!(map[60].len(), 1, "L/R 两层应合并为一层");
        let info = &map[60][0];
        assert!(info.is_stereo);
        assert_eq!(&*info.sample_data, &[0.5, 0.1, 0.6, 0.2]);
        assert_eq!((info.pan_l, info.pan_r), (1.0, 1.0));
    }

    /// 同 (key, vel) 的多个 region（叠加强度层 / SF2 的 L/R 拆分）必须全部返回；
    /// 只取一层会让鼓偏侧、旋律丢泛音（回归：鼓右声道静音）。
    #[test]
    fn select_key_infos_returns_all_layers() {
        let mut map = vec![Vec::new(); 128];
        map[60] = vec![
            KeyInfo {
                lovel: 100,
                hivel: 100,
                volume: 1.0,
                ..Default::default()
            },
            KeyInfo {
                lovel: 100,
                hivel: 100,
                volume: 2.0,
                ..Default::default()
            },
            KeyInfo {
                lovel: 101,
                hivel: 101,
                volume: 3.0,
                ..Default::default()
            },
        ];
        let infos = select_key_infos(&map, 60, 100).expect("精确命中");
        assert_eq!(infos.len(), 2, "同 vel 的两层都要返回");
        assert_eq!(infos[0].volume, 1.0);
        assert_eq!(infos[1].volume, 2.0);

        // 兜底：vel=99 → 最近 vel=100，返回该 vel 的全部层（不是只取一层）
        let infos = select_key_infos(&map, 60, 99).expect("最近层兜底");
        assert_eq!(infos.len(), 2);
        // 单层便捷版仍取首层（诊断/测试用）
        assert_eq!(select_key_info(&map, 60, 100).map(|i| i.volume), Some(1.0));
    }

    /// SF2 立体声 region 被拆成同一样本的硬 L/R 两个 zone：合并为一层原样
    /// 立体声（回归：THDrum 整库鼓声右声道 RMS=0）。
    #[test]
    fn merges_stereo_left_right_layers_sharing_sample() {
        let sample: Arc<[f32]> = Arc::from(vec![0.5, 0.1, 0.6, 0.2]);
        let mut map = vec![Vec::new(); 128];
        map[60] = vec![
            KeyInfo {
                lovel: 60,
                hivel: 60,
                pan_l: 1.0,
                pan_r: 0.0,
                is_stereo: true,
                sample_data: Arc::clone(&sample),
                ..Default::default()
            },
            KeyInfo {
                lovel: 60,
                hivel: 60,
                pan_l: 0.0,
                pan_r: 1.0,
                is_stereo: true,
                sample_data: sample,
                ..Default::default()
            },
        ];
        merge_stereo_pairs(&mut map);
        assert_eq!(map[60].len(), 1, "同一样本的 L/R 两个 zone 应合并为一层");
        let info = &map[60][0];
        assert!(info.is_stereo);
        assert_eq!((info.pan_l, info.pan_r), (1.0, 1.0));
        assert_eq!(&*info.sample_data, &[0.5, 0.1, 0.6, 0.2]);
    }

    /// 参数不一致的左右层（不同音量/包络）不能误合并。
    #[test]
    fn does_not_merge_layers_with_different_params() {
        let mut map = vec![Vec::new(); 128];
        map[60] = vec![
            KeyInfo {
                lovel: 60,
                hivel: 60,
                pan_l: 1.0,
                pan_r: 0.0,
                volume: 1.0,
                sample_data: Arc::from(vec![0.5f32]),
                ..Default::default()
            },
            KeyInfo {
                lovel: 60,
                hivel: 60,
                pan_l: 0.0,
                pan_r: 1.0,
                volume: 0.5,
                sample_data: Arc::from(vec![0.1f32]),
                ..Default::default()
            },
        ];
        merge_stereo_pairs(&mut map);
        assert_eq!(map[60].len(), 2);
    }

    /// SFZ 单乐器库没有 bank 128：鼓通道（bank 128）应回退到普通 bank，
    /// 而不是静音。GM 库有真鼓 bank 时不能被回退污染。
    #[test]
    fn percussion_falls_back_to_bank0_when_soundfont_has_no_drum_bank() {
        let mut sfz_map = vec![Vec::new(); 128];
        sfz_map[60] = vec![KeyInfo {
            lovel: 0,
            hivel: 127,
            volume: 1.0,
            sample_data: Arc::from(vec![0.5f32]),
            ..Default::default()
        }];
        let sfz_only = vec![KeyMapEntry::expanded(0, 0, sfz_map.clone())];
        assert!(
            select_key_infos_multi(&sfz_only, 128, 0, 60, 100).is_some(),
            "SFZ 鼓通道应回退到 bank 0"
        );

        let mut drum_map = vec![Vec::new(); 128];
        drum_map[60] = vec![KeyInfo {
            lovel: 0,
            hivel: 127,
            volume: 0.5,
            sample_data: Arc::from(vec![0.25f32]),
            ..Default::default()
        }];
        let with_drum = vec![
            KeyMapEntry::expanded(0, 0, sfz_map),
            KeyMapEntry::expanded(128, 0, drum_map),
        ];
        let infos =
            select_key_infos_multi(&with_drum, 128, 0, 60, 100).expect("GM 库的鼓 bank 应命中");
        assert_eq!(infos[0].volume, 0.5, "有真鼓 bank 时不应回退到 bank 0");
    }
}
