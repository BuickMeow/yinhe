//! YinModel + TrackData + ConductorData + ProjectMeta + rebuild logic.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::audio_warp::WarpMap;
use crate::clip::Clip;
use crate::events::NoteEvent;
use crate::tempo_map::{TempoMap, TempoSegment, mpq_from_bpm, recompute_tempo_start_times};
use yinhe_types::{AutomationLane, AutomationTarget, KEY_COUNT, NoteBucket, PcEvent};

// =========================================================
//  Conductor
// =========================================================

/// Global score-level events.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConductorData {
    /// Tempo 走 AutomationLane（与 CC/PB/RPN 一致），存于 `target == AutomationTarget::Tempo`。
    /// `events[i].value` 直接装 bpm（f32）。
    pub tempo: AutomationLane,
    pub time_sig: Vec<yinhe_types::TimeSigEvent>,
    /// 调号事件（MIDI Meta FF 59）。全局级，放 conductor。
    #[serde(default)]
    pub key_sig: Vec<yinhe_types::KeySigEvent>,
    /// 标记事件（MIDI Meta FF 06/07）。全局级，放 conductor。
    #[serde(default)]
    pub markers: Vec<yinhe_types::MarkerEvent>,
    /// 歌词事件（MIDI Meta FF 05）。SMF 允许歌词放在 track 0（conductor），
    /// 也可放在普通轨。这里收集 track 0 的歌词；普通轨歌词仍在 TrackData.lyrics。
    #[serde(default)]
    pub lyrics: Vec<yinhe_types::LyricsEvent>,
    /// 和弦事件（非 SMF 标准，仅在 .yin 格式存活）。
    /// conductor 级和弦，per-track 和弦在 TrackData.chord。
    #[serde(default)]
    pub chord: Vec<yinhe_types::ChordEvent>,
}

impl Default for ConductorData {
    fn default() -> Self {
        Self {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: Vec::new(),
            },
            time_sig: Vec::new(),
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        }
    }
}

// =========================================================
//  TrackData
// =========================================================

/// 音轨默认颜色（未设置颜色事件时的占位值，UI 显示时回退到调色板）。
/// RGBA，alpha 默认不透明。
pub const DEFAULT_TRACK_COLOR: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

/// 音轨种类。MIDI 轨按 port/channel（A01..P16，256 通道）路由到该通道的乐器：
/// 默认 XSynth（内置），可挂载 CLAP/VST3 插件乐器覆盖；
/// 音频轨走内嵌音频素材的片段回放（按 audio_channel 路由，独立命名空间）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    #[default]
    Midi,
    Audio,
    /// 非 MIDI 的 Master/Conductor 轨：只承载 Tempo、拍号等全局事件，
    /// 不能有音符、不能挂非 Tempo 自动化，不参与混音的 MIDI 通道路由。
    Master,
}

// =========================================================
//  Audio source / clip
// =========================================================

/// 工程内嵌的音频素材（导入或录音产生）。
///
/// `data` 是**原始文件字节**（wav/mp3/flac/ogg/m4a），随 `.yin` 存档内嵌，
/// 保证工程自包含可移植。运行时解码后的 PCM 与波形峰值是派生数据，
/// 不放进模型（见 yinhe-audio 的 `AudioLibrary`）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioSource {
    /// 工程内唯一 id（片段用 `AudioClip::source` 引用）。
    pub uuid: String,
    /// 显示名（原文件名）。
    pub name: String,
    /// 原始文件字节。`Arc` 共享，编辑模型时不做深拷贝。
    pub data: Arc<Vec<u8>>,
    /// 原始时长（秒），供 UI 显示/裁剪上限。
    pub duration_seconds: f64,
}

/// 音频片段：在绝对时间轴上播放素材的一段（时间基准 = 秒，不受 BPM 影响）。
///
/// 所有时间都相对工程时间线，与 tempo 无关（用户选定的语义）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    /// 工程内唯一片段 id（选择/undo 引用）。
    pub id: u32,
    /// 引用的 `AudioSource::uuid`。
    pub source: String,
    /// 时间线起点（秒）。
    pub start_seconds: f64,
    /// 素材内起点偏移（秒）。
    pub offset_seconds: f64,
    /// 播放时长（秒）。
    pub duration_seconds: f64,
    /// 线性增益（1.0 = 0 dB）。
    #[serde(default = "default_audio_gain")]
    pub gain: f32,
    /// 淡入时长（秒，0 = 无）。
    #[serde(default)]
    pub fade_in_seconds: f64,
    /// 淡出时长（秒，0 = 无）。
    #[serde(default)]
    pub fade_out_seconds: f64,
    /// 反向播放（只翻标志，不改 PCM）。
    #[serde(default)]
    pub reversed: bool,
    /// 时间线起点（beat，工程起点为 0）。`Some` = beat 主时基；旧工程为 None
    /// （秒域原样播放，加载后由 `rebuild_tempo_map` 自动迁移）。
    #[serde(default)]
    pub start_beat: Option<f64>,
    /// beat ↔ 素材秒数的 warp 映射；`None` = 尚未迁移的旧工程。
    #[serde(default)]
    pub warp: Option<WarpMap>,
}

const fn default_audio_gain() -> f32 {
    1.0
}

impl AudioClip {
    /// 时间线终点（秒）。
    pub fn end_seconds(&self) -> f64 {
        self.start_seconds + self.duration_seconds
    }

    /// 是否已进入 beat 主时基（M1 warp 生效）。
    pub fn is_warped(&self) -> bool {
        self.start_beat.is_some() && self.warp.is_some()
    }

    /// 素材内秒数总跨度（warp 前的主时基长度）。
    pub fn source_span_seconds(&self) -> f64 {
        self.warp
            .as_ref()
            .map(|w| w.source_span())
            .unwrap_or(self.duration_seconds)
    }

    /// 只更新时间线起点（移动片段）：保留 warp 的 beat/source 跨度，
    /// 不破坏已有的变速比例。
    pub fn sync_start_beat(&mut self, tempo_map: &TempoMap, ppq: u32) {
        let ppq = ppq.max(1) as f64;
        self.start_beat = Some(tempo_map.tick_at_time(self.start_seconds.max(0.0)) / ppq);
    }

    /// 用当前 tempo map 把秒域时间（用户刚编辑过的值）写回 beat 锚点。
    ///
    /// 用于创建 / 裁剪 / 拆分等改变了时长的操作：自动 warp（`warp.auto`）
    /// 会按新的时长重建两端点映射；自定义 marker 只更新起点，保留已有映射
    /// （后续 warp marker UI 会负责细分编辑）。
    pub fn sync_beat_from_seconds(&mut self, tempo_map: &TempoMap, ppq: u32) {
        let ppq = ppq.max(1) as f64;
        let start = self.start_seconds.max(0.0);
        let end = self.end_seconds().max(start);
        let start_beat = tempo_map.tick_at_time(start) / ppq;
        let end_beat = tempo_map.tick_at_time(end) / ppq;
        let beat_span = (end_beat - start_beat).max(f64::EPSILON);
        self.start_beat = Some(start_beat);
        match &self.warp {
            Some(w) if !w.auto => {
                // 自定义 warp：保留 marker，不在这里重塑。
                let _ = w;
            }
            _ => {
                self.warp = Some(WarpMap::identity(
                    beat_span,
                    self.offset_seconds.max(0.0),
                    self.duration_seconds.max(0.0),
                    true,
                ));
            }
        }
    }

    /// 用 beat 锚点 + warp 重新计算秒域时间（tempo 变化后调用）。
    pub fn sync_seconds_from_beat(&mut self, tempo_map: &TempoMap) {
        let (Some(start_beat), Some(warp)) = (self.start_beat, &self.warp) else {
            return;
        };
        let beat_span = warp.beat_span();
        if beat_span <= 0.0 {
            return;
        }
        let ppq = tempo_map.ticks_per_beat.max(1) as f64;
        let start_tick = (start_beat.max(0.0) * ppq).round() as u64;
        let end_tick = ((start_beat.max(0.0) + beat_span) * ppq).round() as u64;
        let start_sec = tempo_map.tick_to_seconds(start_tick);
        let end_sec = tempo_map.tick_to_seconds(end_tick.max(start_tick + 1));
        self.start_seconds = start_sec;
        self.duration_seconds = (end_sec - start_sec).max(0.001);
        if let Some(first) = warp.markers.first() {
            self.offset_seconds = first.sample_seconds;
        }
    }

    /// 播放进度（0..1）→ 素材秒数（warp 生效时按 marker 分段映射）。
    pub fn source_seconds_at_progress(&self, progress: f64, reversed: bool) -> Option<f64> {
        let warp = self.warp.as_ref()?;
        let p = progress.clamp(0.0, 1.0);
        let beat = if reversed {
            warp.beat_span() * (1.0 - p)
        } else {
            warp.beat_span() * p
        };
        Some(warp.source_seconds_at(beat))
    }
}

/// 计算片段的有效淡入/淡出时长（秒）：合并自身参数与同轨重叠片段的自动交叉淡化。
///
/// 自动交叉淡化规则（同轨两两重叠）：
/// - 另一片段在本片段内部结束（`o.end < c.end`）→ 本片段从自身起点淡入到 `o.end`；
/// - 另一片段在本片段内部开始（`o.start > c.start`）→ 本片段从 `o.start` 淡出到自身终点。
///
/// 同起点重叠不交叉（并排叠加）。结果不超过片段时长。
pub fn effective_fades(clips: &[AudioClip], index: usize) -> (f64, f64) {
    let Some(c) = clips.get(index) else {
        return (0.0, 0.0);
    };
    let mut fade_in = c.fade_in_seconds.max(0.0);
    let mut fade_out = c.fade_out_seconds.max(0.0);
    for (j, o) in clips.iter().enumerate() {
        if j == index {
            continue;
        }
        let overlap_start = c.start_seconds.max(o.start_seconds);
        let overlap_end = c.end_seconds().min(o.end_seconds());
        if overlap_end <= overlap_start {
            continue;
        }
        if o.start_seconds > c.start_seconds {
            fade_out = fade_out.max(c.end_seconds() - overlap_start);
        }
        if o.end_seconds() < c.end_seconds() {
            fade_in = fade_in.max(overlap_end - c.start_seconds);
        }
    }
    (
        fade_in.min(c.duration_seconds.max(0.0)),
        fade_out.min(c.duration_seconds.max(0.0)),
    )
}

/// 音频片段在播放进度 `position_seconds` 处的线性淡入淡出增益（0..1）。
///
/// 渲染、波形绘制与手柄拖动共用这一份端点语义（M0 统一实现）。
pub fn clip_fade_gain(
    position_seconds: f64,
    duration_seconds: f64,
    fade_in_seconds: f64,
    fade_out_seconds: f64,
) -> f32 {
    if duration_seconds <= 0.0 {
        return 0.0;
    }
    let p = position_seconds.clamp(0.0, duration_seconds);
    let mut gain = 1.0;
    if fade_in_seconds > 0.0 && p < fade_in_seconds {
        gain *= (p / fade_in_seconds).clamp(0.0, 1.0);
    }
    let remain = duration_seconds - p;
    if fade_out_seconds > 0.0 && remain < fade_out_seconds {
        gain *= (remain / fade_out_seconds).clamp(0.0, 1.0);
    }
    gain as f32
}

/// One MIDI track's complete data.
///
/// Channel/track are held here, not in individual events. NoteEvent is
/// looked up by `(track_idx, key, note.id)`.
///
/// Control events (CC, PitchBend, RPN, NRPN) are unified into
/// `automation_lanes` — one lane per parameter per track. Program Change
/// is stored separately since it is a discrete event, not a continuous
/// automation curve.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackData {
    pub uuid: String,
    pub name: String,
    /// 音轨颜色（RGBA，0..1）。是 ImageToMidi 颜色事件的来源/去向。
    pub color: [f32; 4],
    /// MIDI port (0..16, displayed as A..P).
    pub port: u8,
    /// MIDI channel (0..16, displayed as 1..16).
    pub channel: u8,
    pub channel_prefix: Option<u8>,
    pub muted: bool,
    pub soloed: bool,
    /// 音轨种类（旧存档无此字段，默认 Midi）。
    #[serde(default)]
    pub kind: TrackKind,
    /// 音频通道号（0 起，UI 显示 1 起）。仅音频轨有意义；
    /// 与 MIDI/乐器通道均无关，是音频混音条的路由命名空间。
    /// 多条音频轨共享同一音频通道 = 共享同一条混音 strip/insert 链。
    #[serde(default)]
    pub audio_channel: Option<u16>,
    /// 音频片段（仅音频轨有意义），按 start_seconds 排序由编辑命令维护。
    #[serde(default)]
    pub audio_clips: Vec<AudioClip>,
    /// MIDI Clip v1：该轨的命名时间区间，按 start_tick 排序且互不重叠。
    #[serde(default)]
    pub clips: Vec<Clip>,

    /// Notes are stored in `YinModel.notes` (by-key store).
    /// This field is only used during parsing and is moved out
    /// by `YinModel::load_track_notes`. At runtime it is empty.
    pub notes: Vec<NoteEvent>,
    /// Unified automation lanes: CC, PitchBend, RPN, NRPN.
    /// Each lane is a sorted list of (tick, value) events.
    pub automation_lanes: Vec<yinhe_types::AutomationLane>,
    /// Program Change events (discrete, not automation).
    pub program_change: Vec<PcEvent>,
    /// 歌词事件（MIDI Meta FF 05）。SMF 标准中歌词是 per-track 的。
    #[serde(default)]
    pub lyrics: Vec<yinhe_types::LyricsEvent>,
    /// 和弦事件（非 SMF 标准，用 Text meta 存储）。per-track。
    #[serde(default)]
    pub chord: Vec<yinhe_types::ChordEvent>,
}

impl TrackData {
    pub fn new(port: u8, channel: u8) -> Self {
        Self {
            uuid: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            color: DEFAULT_TRACK_COLOR,
            port,
            channel,
            channel_prefix: None,
            muted: false,
            soloed: false,
            kind: TrackKind::Midi,
            audio_channel: None,
            audio_clips: Vec::new(),
            clips: Vec::new(),
            notes: Vec::new(),
            automation_lanes: Vec::new(),
            program_change: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        }
    }

    /// Global channel = port * 16 + channel (0..255).
    pub fn global_channel(&self) -> u8 {
        (self.port & 0x0F) << 4 | (self.channel & 0x0F)
    }
}

// =========================================================
//  Project metadata
// =========================================================

/// 工程内音乐人引用（有序；第一项为主音乐人）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    /// 全局音乐人档案 id（可空；空 = 仅手填名字）。
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// 角色（如 "main" / "feat."），空 = 未指定。
    #[serde(default)]
    pub role: String,
    /// 个人网站/主页（可多条，仅文本，不嵌入头像）。
    #[serde(default)]
    pub websites: Vec<String>,
    /// 个人介绍（仅文本）。
    #[serde(default)]
    pub bio: String,
}

/// 工程最终封面（仅一张；`.yin` 独立段存储）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverArt {
    pub mime: String,
    pub data: Arc<Vec<u8>>,
    /// 列表/库视图用的小图；可为空。
    pub thumb: Arc<Vec<u8>>,
}

/// 歌曲元数据（可编辑文本字段 + 编号 + 音乐人列表）。
///
/// 与 `ProjectMeta` 同步；单独成结构体用于工程设置对话框一次性比较/提交，
/// 避免为每个字段各建一个 undo action。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongInfo {
    pub name: String,
    /// 兼容字段：MIDI Copyright / 单字段导出取首位艺术家。
    pub artist: String,
    pub description: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: String,
    pub track_no: u16,
    pub disc_no: u16,
    /// 兼容字段：单字段音频标签取合并后的艺术家文本。
    pub copyright: String,
    /// 唯一来源：艺术家 / 音乐人 / 版权统一为有序列表。
    pub artists: Vec<ArtistRef>,
}

/// 单字段格式（音频标签、MIDI 兼容字段）使用的合并文本。
fn join_artist_names(artists: &[ArtistRef]) -> String {
    artists
        .iter()
        .map(|a| a.name.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" / ")
}

impl SongInfo {
    pub fn from_meta(meta: &ProjectMeta) -> Self {
        // 艺术家 / 音乐人 / 版权统一为一组：旧工程的 artist 与 copyright
        // 都迁移成独立艺术家框，保存后只剩 artists 作为唯一来源。
        let mut artists = meta.artists.clone();
        let effective = join_artist_names(&artists);
        for legacy in [&meta.artist, &meta.copyright] {
            let name = legacy.trim();
            if name.is_empty()
                || artists.iter().any(|a| a.name.trim() == name)
                || (!effective.is_empty() && name == effective)
            {
                continue;
            }
            artists.push(ArtistRef {
                id: String::new(),
                name: name.to_string(),
                role: String::new(),
                websites: Vec::new(),
                bio: String::new(),
            });
        }
        let copyright = if artists.is_empty() {
            meta.copyright.clone()
        } else {
            join_artist_names(&artists)
        };
        Self {
            name: meta.name.clone(),
            artist: meta.artist.clone(),
            description: meta.description.clone(),
            album: meta.album.clone(),
            album_artist: meta.album_artist.clone(),
            genre: meta.genre.clone(),
            year: meta.year.clone(),
            track_no: meta.track_no,
            disc_no: meta.disc_no,
            copyright,
            artists,
        }
    }

    /// 导出标签用的音乐人文本：优先有序列表，列表为空回退旧 `artist`。
    pub fn effective_artist(&self) -> String {
        if self.artists.is_empty() {
            return self.artist.clone();
        }
        join_artist_names(&self.artists)
    }

    pub fn apply_to(&self, meta: &mut ProjectMeta) {
        meta.name.clone_from(&self.name);
        // artist / copyright 是兼容字段：artist 取首位，copyright 取合并文本，
        // 因为 MIDI 和音频标签都只有一个字段可写。
        meta.artist = self
            .artists
            .iter()
            .find(|a| !a.name.trim().is_empty())
            .map(|a| a.name.trim().to_string())
            .unwrap_or_else(|| self.artist.clone());
        meta.description.clone_from(&self.description);
        meta.album.clone_from(&self.album);
        meta.album_artist.clone_from(&self.album_artist);
        meta.genre.clone_from(&self.genre);
        meta.year.clone_from(&self.year);
        meta.track_no = self.track_no;
        meta.disc_no = self.disc_no;
        meta.copyright = if self.artists.is_empty() {
            self.copyright.clone()
        } else {
            self.effective_artist()
        };
        meta.artists.clone_from(&self.artists);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    /// 兼容字段：由 `artists` 首位同步。
    pub artist: String,
    pub description: String,
    pub ppq: u32,
    pub compression_level: i32,
    /// 本工程默认 BPM：conductor 没有原生 Tempo 事件时的回退值
    /// （原生事件优先级更高，可以完全代替它）。
    #[serde(default = "default_project_bpm")]
    pub default_bpm: f32,
    /// 本工程默认拍号 `(分子, 分母的 2 次幂)`，如 4/4 = `(4, 2)`：
    /// conductor 没有原生拍号事件时的回退值（原生事件优先级更高）。
    #[serde(default = "default_project_time_sig")]
    pub default_time_sig: (u8, u8),

    // ── 歌曲信息（旧存档缺失时为空/0）──
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_artist: String,
    #[serde(default)]
    pub genre: String,
    /// 发行年份/日期文本（保持字符串，兼容 "2026"、"2026-11"）。
    #[serde(default)]
    pub year: String,
    /// 音轨号（0 = 未设置）。
    #[serde(default)]
    pub track_no: u16,
    /// 碟号（0 = 未设置）。
    #[serde(default)]
    pub disc_no: u16,
    #[serde(default)]
    pub copyright: String,
    /// 工程音乐人列表（有序；第一项为主音乐人）。
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
    /// 累计前台工作时间（秒）；只在保存时落盘，不计入 undo。
    #[serde(default)]
    pub work_time_secs: u64,
}

fn default_project_bpm() -> f32 {
    120.0
}

fn default_project_time_sig() -> (u8, u8) {
    (4, 2)
}

impl Default for ProjectMeta {
    fn default() -> Self {
        Self {
            name: String::new(),
            artist: String::new(),
            description: String::new(),
            ppq: 480,
            compression_level: 3,
            default_bpm: default_project_bpm(),
            default_time_sig: default_project_time_sig(),
            album: String::new(),
            album_artist: String::new(),
            genre: String::new(),
            year: String::new(),
            track_no: 0,
            disc_no: 0,
            copyright: String::new(),
            artists: Vec::new(),
            work_time_secs: 0,
        }
    }
}

// =========================================================
//  YinModel
// =========================================================

/// The single source of truth for a Yinhe project.
///
/// All note data lives in the `notes` array (by key, sorted by start_tick).
/// `TrackData` no longer stores notes — they are accessed via `notes[key]`
/// and filtered by `note.track` when per-track iteration is needed.
///
/// Tracks are held in `Arc<TrackData>` for cheap clone-on-write editing
/// (C1 mode). The conductor is also Arc to avoid copying it when only
/// a track changes. tempo_map is derived from conductor.
/// 通道级默认乐器：没有 MIDI PC 时生效的全局 PC。
/// 同一 MIDI 通道的所有音轨共享；任一 PC 事件都会覆盖它。
///
/// 三个分量对应 MIDI 的 Bank Select + Program Change 三元组：
/// 很多 SF2/SFZ 音色库用 bank 区分同一 program 的不同音色，
/// 因此 bank 的 MSB / LSB 必须能独立设置，不能绑在 program 下拉上。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelInstrument {
    /// Bank Select MSB（CC0，0..=127）。
    #[serde(default)]
    pub bank_msb: u8,
    /// Bank Select LSB（CC32，0..=127）。
    #[serde(default)]
    pub bank_lsb: u8,
    /// Program / Preset（0..=127）。
    pub program: u8,
    /// 鼓通道：GM percussion（bank 128 语义），由合成器的 PercussionMode 声明。
    ///
    /// 独立成字段而不是复用 `bank_msb == 128`：bank 是音色库维度，
    /// 鼓是通道模式，二者混在一起会让「用 bank 128 的非鼓音色」无法表达。
    #[serde(default)]
    pub is_drum: bool,
}

impl ChannelInstrument {
    /// 旧工程的兼容读法：历史上鼓通道用 `bank_msb == 128` 表示。
    ///
    /// 加载 mapping 时调用一次即可迁移到显式 `is_drum`，
    /// 迁移后 `bank_msb` 归 0（鼓组不占用真实 bank 号）。
    pub fn normalize_legacy_bank(&mut self) {
        if self.bank_msb >= 128 {
            self.is_drum = true;
            self.bank_msb = 0;
        }
    }
}

#[derive(Clone, Debug)]
pub struct YinModel {
    pub conductor: Arc<ConductorData>,
    pub tracks: Vec<Arc<TrackData>>,
    pub tempo_map: Arc<TempoMap>,
    pub meta: ProjectMeta,

    /// 16 个 MIDI 通道的默认乐器（None = 不覆盖，按 MIDI 内 PC / program 0）。
    pub channel_instruments: [Option<ChannelInstrument>; 16],

    /// 内置音源的可调参数（按全局通道 0..256）。默认 = 不改变音色库。
    /// 持久化在 `mapping.json`（自描述 JSON），不进 data 段的 postcard。
    pub synth_params: Vec<yinhe_types::SynthParams>,

    /// Single authoritative note store: `notes[key]` = all notes at that key,
    /// sorted by start_tick (chunked, 65536 notes/chunk). Each note carries
    /// its track index and全局唯一 id。
    /// Compatible with `yinhe_types::NoteSource`.
    pub notes: Box<[Arc<NoteBucket>; KEY_COUNT]>,
    pub note_count: u64,
    pub tick_length: u64,
    /// 全曲最长音符长度（tick），只增不减：删除最长音符后保留旧值只会让
    /// 视口查询左边界略偏左，绝不漏音符。由 `load_track_notes` / `rebuild` /
    /// `rebuild_dirty` / `rescale_ppq` 维护；供 `NoteSource::max_note_len` 返回。
    pub max_note_len: u32,
    /// Per-track note count cache (avoids scanning KEY_COUNT buckets for stats).
    pub track_note_count: Vec<u64>,
    /// Per-track audible note count (velocity > 1). `> 0` means the track
    /// has at least one audible note. Replaces the old `track_has_audio_cache: Vec<bool>`.
    pub track_audible_count: Vec<u64>,

    /// Dirty bucket tracking: `dirty_keys[k]` is true when bucket k has been
    /// modified and needs incremental stats. Buckets are kept sorted by the
    /// write paths themselves (`rebuild_dirty` no longer sorts). Use
    /// `mark_dirty()` to set, `rebuild_dirty()` to clear. Public for struct
    /// construction via `..Default::default()`.
    pub dirty_keys: [bool; KEY_COUNT],

    /// Per-key note revision counter. Bumped every time `mark_dirty(k)` is
    /// called, **not** cleared by `rebuild_dirty()`. Consumers (e.g. GPU cull
    /// buffer upload) compare these to detect which keys need incremental
    /// re-upload without rescanning all KEY_COUNT buckets.
    pub note_revisions: [u64; KEY_COUNT],

    /// Per-bucket note count cache for O(D) incremental stats in `rebuild_dirty()`.
    /// Updated by `rebuild()`, `load_track_notes()`, and `rebuild_dirty()`.
    pub bucket_note_count: [u64; KEY_COUNT],

    /// Per-bucket max end_tick cache. Enables correct incremental `tick_length`
    /// in `rebuild_dirty()`: shrink-aware (max over all KEY_COUNT buckets) without
    /// O(N) full scan. Updated by `rebuild()`, `load_track_notes()`, and `rebuild_dirty()`.
    pub bucket_max_end_tick: [u64; KEY_COUNT],

    /// Per-bucket per-track (total, audible) counts. Sparse: each bucket
    /// only stores tracks that actually have notes in it. Enables
    /// O(dirty bucket size) incremental `track_note_count` /
    /// `track_audible_count` updates in `rebuild_dirty()` instead of
    /// rescanning all KEY_COUNT buckets.
    pub bucket_track_stats: [HashMap<u16, (u64, u64)>; KEY_COUNT],

    /// 全局音符 id 发号器（下一个待分配的 id）。
    /// 0 保留为"未分配"哨兵，实际 id 从 1 开始。
    /// 编辑时调 `alloc_note_id()`，加载时由 `load_track_notes` 统一分配。
    pub next_note_id: u32,
    /// 全局 Clip id 发号器（0 保留哨兵，实际从 1 开始）。
    pub next_clip_id: u32,

    /// 工程内嵌音频素材表（去重：一个素材可被多个片段引用）。
    /// `Arc` 共享，编辑模型时不做字节深拷贝。
    pub audio_sources: Vec<Arc<AudioSource>>,
    /// 全局音频片段 id 发号器（0 保留为哨兵，实际从 1 开始）。
    pub next_audio_clip_id: u32,

    /// 全局自动化事件 id 发号器（0 保留为哨兵，实际从 1 开始）。
    /// 编辑时调 `alloc_automation_id()`，加载后由
    /// [`YinModel::renumber_automation_ids`] 统一分配（id 不落盘）。
    pub next_automation_id: u32,
}

impl Default for YinModel {
    fn default() -> Self {
        Self {
            conductor: Arc::new(ConductorData::default()),
            tracks: Vec::new(),
            tempo_map: Arc::new(TempoMap::default()),
            meta: ProjectMeta::default(),
            channel_instruments: [None; 16],
            synth_params: vec![yinhe_types::SynthParams::default(); 256],
            notes: Box::new(core::array::from_fn(|_| Arc::new(NoteBucket::default()))),
            note_count: 0,
            tick_length: 0,
            max_note_len: 0,
            track_note_count: Vec::new(),
            track_audible_count: Vec::new(),
            dirty_keys: [false; KEY_COUNT],
            note_revisions: [0; KEY_COUNT],
            bucket_note_count: [0; KEY_COUNT],
            bucket_max_end_tick: [0; KEY_COUNT],
            bucket_track_stats: core::array::from_fn(|_| HashMap::new()),
            next_note_id: 1,
            next_clip_id: 1,
            audio_sources: Vec::new(),
            next_audio_clip_id: 1,
            next_automation_id: 1,
        }
    }
}

impl YinModel {
    /// 新建一个空工程：Conductor 轨（120 BPM + 4/4 拍号）+ 16 条 Track 1..Track 16 轨道。
    ///
    /// 工厂方法封装了 `Document::empty()` 之前内联构造的样板代码，
    /// 避免 document.rs 直接操作 YinModel 内部字段。
    /// 调用者拿到模型后自行决定是否 `rebuild()`（首次构造不需要，
    /// 因为没有音符；但 `Document::empty()` 仍会调用以初始化 tempo_map）。
    pub fn new_empty_with_16_tracks() -> Self {
        // 新工程不写原生 Tempo/拍号事件：走 `ProjectMeta` 的默认值
        // （120 BPM + 4/4），原生事件优先级更高，用户插入事件即覆盖。
        let conductor = Arc::new(ConductorData::default());

        let mut tracks: Vec<Arc<TrackData>> = Vec::with_capacity(17);
        let mut t = TrackData::new(0, 0);
        t.name = "Conductor".to_string();
        t.kind = TrackKind::Master;
        tracks.push(Arc::new(t));
        for ch in 0..16u8 {
            let mut t = TrackData::new(0, ch);
            // 轨道号 = 在 tracks 中的位置（Conductor 占 0 号，音轨从 1 号开始）。
            // 不按通道命名：通道经常被改，轨道号相对固定。
            t.name = format!("Track {}", tracks.len());
            tracks.push(Arc::new(t));
        }

        // A10（1-based 第 10 通道，索引 9）默认是鼓通道。用显式 `is_drum`
        // 表示鼓，不占用 bank 号。任一 MIDI PC 仍可覆盖。
        let mut channel_instruments = [None; 16];
        channel_instruments[9] = Some(ChannelInstrument {
            bank_msb: 0,
            bank_lsb: 0,
            program: 0,
            is_drum: true,
        });

        Self {
            conductor,
            tracks,
            channel_instruments,
            ..Default::default()
        }
    }

    /// 全局通道的内置音源参数（越界 = 默认值）。
    pub fn synth_params_at(&self, channel: usize) -> yinhe_types::SynthParams {
        self.synth_params.get(channel).copied().unwrap_or_default()
    }

    /// 可变的全局通道参数；通道越界时自动扩展。
    pub fn synth_params_mut(&mut self, channel: usize) -> &mut yinhe_types::SynthParams {
        if channel >= self.synth_params.len() {
            self.synth_params
                .resize(channel + 1, yinhe_types::SynthParams::default());
        }
        &mut self.synth_params[channel]
    }

    /// Build TempoMap from conductor.tempo / conductor.time_sig.
    ///
    /// 没有原生事件时用 `meta.default_bpm` / `meta.default_time_sig` 兜底
    /// （原生事件优先级更高）。
    pub(crate) fn build_tempo_map(&self) -> TempoMap {
        let ppq = self.meta.ppq;
        let default_mpq = mpq_from_bpm(self.meta.default_bpm);
        let tick_length = self.tick_length;

        // 排序 + 同 tick 去重（保留最后一个，与 MIDI 导入一致）。
        let mut sorted: Vec<(u32, f32)> = self
            .conductor
            .tempo
            .events
            .iter()
            .map(|e| (e.tick, e.value))
            .collect();
        sorted.sort_by_key(|e| e.0);
        let mut tempo_events: Vec<(u32, f32)> = Vec::with_capacity(sorted.len());
        for e in sorted {
            if tempo_events.last().is_some_and(|prev| prev.0 == e.0) {
                tempo_events.pop();
            }
            tempo_events.push(e);
        }

        // 0 BPM = 剪切点：从第一个 0 直接跳到下一个有效 BPM（连续 0 继续跳），
        // 没有有效 BPM 则跳到曲末。`[start, end)` 区间不播放。
        let max_tick = u32::try_from(tick_length).unwrap_or(u32::MAX);
        let mut skip_ranges: Vec<(u32, u32)> = Vec::new();
        let mut i = 0;
        while i < tempo_events.len() {
            if tempo_events[i].1 > 0.0 {
                i += 1;
                continue;
            }
            let start = tempo_events[i].0;
            let mut j = i;
            while j < tempo_events.len() && tempo_events[j].1 <= 0.0 {
                j += 1;
            }
            let end = if j < tempo_events.len() {
                tempo_events[j].0
            } else {
                max_tick
            };
            if end > start {
                skip_ranges.push((start, end));
            }
            i = j;
        }

        // 有效 tempo 段：0 BPM 只作为剪切标记，不产生速度段。
        let mut segments: Vec<TempoSegment> = Vec::new();
        if tempo_events.first().is_none_or(|e| e.0 != 0) {
            segments.push(TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: default_mpq,
            });
        }
        for &(tick, bpm) in &tempo_events {
            if bpm > 0.0 {
                segments.push(TempoSegment {
                    start_tick: tick,
                    start_time: 0.0,
                    micros_per_quarter: mpq_from_bpm(bpm),
                });
            }
        }
        if segments.is_empty() {
            segments.push(TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: default_mpq,
            });
        }

        // 每个剪切段插入一个 mpq=0 的“跳转段”：tick→时间在区间内保持常量；
        // `recompute_tempo_start_times` 会让后续段的 start_time 与跳转点相同。
        for &(start, end) in &skip_ranges {
            let pos = segments.partition_point(|s| s.start_tick <= start);
            segments.insert(
                pos,
                TempoSegment {
                    start_tick: start,
                    start_time: 0.0,
                    micros_per_quarter: 0,
                },
            );
            // 末尾剪切没有下一个 BPM：补一个曲末段，让 tick_at_time 在跳转点
            // 直接返回曲末（否则播放头会停在剪切起点）。
            if end == max_tick && !tempo_events.iter().any(|&(t, bpm)| t == end && bpm > 0.0) {
                let mpq = segments
                    .iter()
                    .rev()
                    .find(|s| s.micros_per_quarter > 0)
                    .map_or(default_mpq, |s| s.micros_per_quarter);
                let pos_end = segments.partition_point(|s| s.start_tick <= end);
                segments.insert(
                    pos_end,
                    TempoSegment {
                        start_tick: end,
                        start_time: 0.0,
                        micros_per_quarter: mpq,
                    },
                );
            }
        }
        recompute_tempo_start_times(&mut segments, ppq);

        // 剪切段在原始时间轴上的秒区间（音频片段用）：跳转点时间 + 被剪
        // tick 在剪切前速度下的时长。
        let mut skip_time_ranges: Vec<(f64, f64)> = Vec::with_capacity(skip_ranges.len());
        for &(start, end) in &skip_ranges {
            let jump_time = segments
                .iter()
                .rev()
                .find(|s| s.start_tick == start && s.micros_per_quarter == 0)
                .map_or(0.0, |s| s.start_time);
            let prev_mpq = segments
                .iter()
                .rev()
                .find(|s| s.start_tick <= start && s.micros_per_quarter > 0)
                .map_or(default_mpq, |s| s.micros_per_quarter);
            let dur = crate::ticks_to_seconds((end - start) as u64, ppq, prev_mpq);
            skip_time_ranges.push((jump_time, jump_time + dur));
        }

        let mut ts_events = self.conductor.time_sig.clone();
        ts_events.sort_by_key(|e| e.tick);

        let time_sig_default = ts_events
            .first()
            .map(|e| (e.numerator, e.denominator))
            .unwrap_or(self.meta.default_time_sig);

        TempoMap {
            ticks_per_beat: ppq,
            tempo_segments: segments,
            time_sig_events: ts_events,
            time_sig_default,
            tick_length,
            skip_ranges,
            skip_time_ranges,
        }
    }

    /// Only rebuild `tempo_map` from `conductor.tempo` / `conductor.time_sig`.
    ///
    /// Use after editing Tempo automation events when notes are untouched.
    /// O(tempo_events + time_sig_events), typically < 100 events — near-instant
    /// even for 100M-note projects. Cheaper than `rebuild()` / `rebuild_dirty()`
    /// which also rescan note buckets.
    pub fn rebuild_tempo_map(&mut self) {
        self.tempo_map = Arc::new(self.build_tempo_map());
        self.sync_audio_clip_seconds();
    }

    /// 用户编辑了音频片段的秒域**时长**（裁剪/拆分/新建）后，按当前 tempo
    /// map 重建 beat 锚点。移动请用 [`Self::sync_audio_clip_starts`]。
    pub fn sync_audio_clip_beats(&mut self) {
        if self.tracks.iter().all(|t| t.audio_clips.is_empty()) {
            return;
        }
        let tempo_map = Arc::clone(&self.tempo_map);
        let ppq = self.meta.ppq.max(1);
        for track in &mut self.tracks {
            if track.audio_clips.is_empty() {
                continue;
            }
            let track = Arc::make_mut(track);
            for clip in &mut track.audio_clips {
                // 已 warp 的片段不在这里重算：它们的时长编辑由具体操作
                // 单独同步，避免把变速比例重置成 1:1。
                if !clip.is_warped() {
                    clip.sync_beat_from_seconds(&tempo_map, ppq);
                }
            }
        }
    }

    /// 只同步起点（移动/复制）：保留每个片段已有的 warp 跨度。
    pub fn sync_audio_clip_starts(&mut self) {
        if self.tracks.iter().all(|t| t.audio_clips.is_empty()) {
            return;
        }
        let tempo_map = Arc::clone(&self.tempo_map);
        let ppq = self.meta.ppq.max(1);
        for track in &mut self.tracks {
            if track.audio_clips.is_empty() {
                continue;
            }
            let track = Arc::make_mut(track);
            for clip in &mut track.audio_clips {
                clip.sync_start_beat(&tempo_map, ppq);
            }
        }
    }

    /// tempo 变化后：beat 主时基 → 秒域。旧工程（未迁移）在这里补上 beat
    /// 锚点，之后就会跟随速度变化。
    fn sync_audio_clip_seconds(&mut self) {
        if self.tracks.iter().all(|t| t.audio_clips.is_empty()) {
            return;
        }
        let tempo_map = Arc::clone(&self.tempo_map);
        let ppq = self.meta.ppq.max(1);
        for track in &mut self.tracks {
            if track.audio_clips.is_empty() {
                continue;
            }
            let track = Arc::make_mut(track);
            for clip in &mut track.audio_clips {
                if clip.is_warped() {
                    clip.sync_seconds_from_beat(&tempo_map);
                } else {
                    // 旧工程：按当前速度锚定一次，之后跟随 tempo 变化。
                    clip.sync_beat_from_seconds(&tempo_map, ppq);
                }
            }
        }
    }

    /// 应用轨号重映射：`remap[旧轨号] = 新轨号`，`u16::MAX` = 删除该轨音符；
    /// 越界旧轨号同样按删除处理（不 panic）。
    ///
    /// 一趟 `retain_mut` 同时完成删除与重编号（track 不是排序键，桶仍有序）。
    /// 与 `mark_dirty` 模式一致：不自动 rebuild，调用方随后调用
    /// [`YinModel::rebuild`] 重建统计与 revision。
    pub fn apply_track_remap(&mut self, remap: &[u16]) {
        // 恒等重映射（末尾追加新轨等）无需遍历任何音符。
        if remap.iter().enumerate().all(|(i, &new)| i as u16 == new) {
            return;
        }
        for bucket in self.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            bucket.retain_mut(|n| match remap.get(n.track as usize) {
                Some(&new) if new != u16::MAX => {
                    n.track = new;
                    true
                }
                _ => false,
            });
        }
    }

    /// 一趟完成轨号重映射并收集被删除的音符（含 key），供 undo 恢复。
    ///
    /// 语义与 [`YinModel::apply_track_remap`] 相同，但额外返回 `(Note, key)`；
    /// 调用方仍需自行重建统计（推荐 [`YinModel::rebuild_stats_only`]，
    /// remap 不破坏桶内排序，无需全量 sort）。
    pub fn apply_track_remap_collect(&mut self, remap: &[u16]) -> Vec<(yinhe_types::Note, u8)> {
        let mut deleted = Vec::new();
        if remap.iter().enumerate().all(|(i, &new)| i as u16 == new) {
            return deleted;
        }
        for (key, bucket) in self.notes.iter_mut().enumerate() {
            let bucket = Arc::make_mut(bucket);
            bucket.retain_mut(|n| match remap.get(n.track as usize) {
                Some(&new) if new != u16::MAX => {
                    n.track = new;
                    true
                }
                _ => {
                    deleted.push((*n, key as u8));
                    false
                }
            });
        }
        deleted
    }

    /// 轨道结构变化后，把每条 automation lane 的 `track` 字段同步为所属轨道
    /// 的当前下标。`lane.track` 用于 AR 自动化曲线取色；插入/删除/移动轨道后
    /// 若不更新，曲线会沿用旧下标对应的颜色。
    pub fn sync_automation_lane_tracks(&mut self) {
        for (i, track) in self.tracks.iter_mut().enumerate() {
            let track = Arc::make_mut(track);
            for lane in &mut track.automation_lanes {
                lane.track = i as u16;
            }
        }
    }

    /// 头部是否为 conductor 轨。优先看显式角色标记（`TrackKind::Master`），
    /// 旧存档无标记时回退到启发式（0 号位无音符、无自动化 lane、无 program
    /// change）。空模型返回 false。
    pub fn has_conductor_track(&self) -> bool {
        if self
            .tracks
            .first()
            .is_some_and(|t| t.kind == TrackKind::Master)
        {
            return true;
        }
        !self.tracks.is_empty()
            && self.track_note_count.first().copied().unwrap_or(0) == 0
            && self
                .tracks
                .first()
                .is_some_and(|t| t.automation_lanes.is_empty() && t.program_change.is_empty())
    }

    /// 确保头部有 conductor 轨：无则在 0 号位插入「Conductor」，并把所有
    /// 音符与自动化 lane 的轨号 +1（幂等；空模型不插入）。已有 conductor
    /// （含旧存档的启发式判定）则补上 `TrackKind::Master` 角色标记。
    ///
    /// 返回是否真的插入了（调用方据此决定是否 `rebuild`；与
    /// [`YinModel::apply_track_remap`] 一致不自动 rebuild）。
    pub fn ensure_conductor_track(&mut self) -> bool {
        if self.tracks.is_empty() {
            return false;
        }
        if self.has_conductor_track() {
            // 已在 0 号位（旧存档无显式标记）：补齐角色，不改动任何数据。
            Arc::make_mut(&mut self.tracks[0]).kind = TrackKind::Master;
            return false;
        }
        let mut conductor = TrackData::new(0, 0);
        conductor.name = "Conductor".to_string();
        conductor.kind = TrackKind::Master;
        for bucket in self.notes.iter_mut() {
            for n in Arc::make_mut(bucket).iter_mut() {
                n.track += 1;
            }
        }
        for track in self.tracks.iter_mut() {
            let track = Arc::make_mut(track);
            for lane in track.automation_lanes.iter_mut() {
                lane.track += 1;
            }
        }
        self.tracks.insert(0, Arc::new(conductor));
        true
    }

    /// Iterate all notes belonging to a specific track.
    ///
    /// Scans all KEY_COUNT key buckets and yields notes where `note.track == track_idx`.
    /// O(N) in total notes. For statistics use `track_note_count[track_idx]`.
    pub fn notes_for_track(&self, track_idx: u16) -> impl Iterator<Item = &yinhe_types::Note> {
        self.notes
            .iter()
            .flat_map(move |bucket| bucket.iter().filter(move |n| n.track == track_idx))
    }

    /// Check if a track has any audible notes (velocity > 1).
    pub fn track_has_audio(&self, track_idx: u16) -> bool {
        self.track_audible_count
            .get(track_idx as usize)
            .copied()
            .unwrap_or(0)
            > 0
    }

    /// 分配一个新的音频片段 id。
    pub fn alloc_audio_clip_id(&mut self) -> u32 {
        let id = self.next_audio_clip_id.max(1);
        self.next_audio_clip_id = id.wrapping_add(1);
        id
    }

    /// 分配一个新的自动化事件 id。
    pub fn alloc_automation_id(&mut self) -> u32 {
        let id = self.next_automation_id.max(1);
        self.next_automation_id = id.wrapping_add(1);
        id
    }

    /// 加载后统一为自动化事件发号（conductor tempo + 每轨每条 lane）。
    ///
    /// id 不落盘（见 `AutomationEvent::id` 注释），旧 `.yin`/MIDI 导入的事件
    /// id 均为 0；加载完成时调用本方法分配会话内唯一 id。
    /// 顺带把同 tick 重复事件去重（保留首个），维持"同一 lane 内 tick 唯一"
    /// 不变量（否则按 tick 定位的编辑/undo 会歧义）。
    pub fn renumber_automation_ids(&mut self) {
        let mut next = 1u32;
        let mut assign = |lane: &mut yinhe_types::AutomationLane| {
            lane.events.dedup_by_key(|e| e.tick);
            for evt in &mut lane.events {
                evt.id = next;
                next = next.wrapping_add(1);
            }
        };
        assign(&mut Arc::make_mut(&mut self.conductor).tempo);
        for track in &mut self.tracks {
            let track = Arc::make_mut(track);
            for lane in &mut track.automation_lanes {
                assign(lane);
            }
        }
        self.next_automation_id = next.max(1);
    }

    /// 按 uuid 查找音频素材。
    pub fn audio_source(&self, uuid: &str) -> Option<&Arc<AudioSource>> {
        self.audio_sources.iter().find(|s| s.uuid == uuid)
    }

    /// 所有音频片段的时间线末尾（秒）。无音频时 0。
    pub fn audio_end_seconds(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| t.audio_clips.iter())
            .map(|c| c.end_seconds())
            .fold(0.0, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_stats::NoteLoader;

    fn note(start: u32, end: u32, key: u8) -> NoteEvent {
        NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: end,
            key,
            velocity: 100,
        }
    }

    fn clip(id: u32, start: f64, dur: f64, fade_in: f64, fade_out: f64) -> AudioClip {
        AudioClip {
            id,
            source: "s".into(),
            start_seconds: start,
            offset_seconds: 0.0,
            duration_seconds: dur,
            gain: 1.0,
            fade_in_seconds: fade_in,
            fade_out_seconds: fade_out,
            reversed: false,
            start_beat: None,
            warp: None,
        }
    }

    #[test]
    fn crossfade_overlap_computes_effective_fades() {
        // A [0,4) 与 B [3,5) 重叠 [3,4)：A 在重叠区淡出 1s，B 从 3s 处淡入到 4s（1s）。
        let clips = vec![clip(1, 0.0, 4.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)];
        assert_eq!(effective_fades(&clips, 0), (0.0, 1.0));
        assert_eq!(effective_fades(&clips, 1), (1.0, 0.0));
        // 自身淡入淡出更大时保留自身。
        let clips = vec![clip(1, 0.0, 4.0, 2.0, 2.0)];
        assert_eq!(effective_fades(&clips, 0), (2.0, 2.0));
        // 不重叠：保持自身（0）。
        let clips = vec![clip(1, 0.0, 2.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)];
        assert_eq!(effective_fades(&clips, 0), (0.0, 0.0));
        assert_eq!(effective_fades(&clips, 1), (0.0, 0.0));
        // 完全包含：外片段获得淡入（到内片段尾）+ 淡出（从内片段头到自身尾）。
        let clips = vec![clip(1, 0.0, 10.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)];
        let (fi, fo) = effective_fades(&clips, 0);
        assert!((fi - 5.0).abs() < 1e-9, "fi={fi}");
        assert!((fo - 7.0).abs() < 1e-9, "fo={fo}");
    }

    #[test]
    fn empty_model_rebuild() {
        let mut m = YinModel::default();
        m.rebuild();
        assert_eq!(m.note_count, 0);
        assert_eq!(m.tick_length, 0);
        assert!(m.notes.iter().all(|v| v.is_empty()));
    }

    #[test]
    fn load_track_notes_assigns_and_preserves_ids() {
        // id=0 自动发号；非 0（旧 .yin）保留原 id 并推进发号器到 max+1
        let mut n0 = note(0, 480, 60);
        n0.id = 0;
        let mut n1 = note(480, 960, 60);
        n1.id = 42;
        let mut n2 = note(960, 1440, 64);
        n2.id = 7;
        let per_track = vec![vec![n0, n1, n2]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);

        // id=0 的从 1 开始分配，不与已保留 id 冲突
        assert_eq!(m.notes[60][0].id, 1);
        assert_eq!(m.notes[60][1].id, 42);
        assert_eq!(m.notes[64][0].id, 7);
        assert_eq!(m.next_note_id, 43, "发号器应推进到 max+1");
    }

    /// remap_track_stats：纯重排时轨道级/桶级统计按 remap 搬移（零音符遍历）。
    #[test]
    fn remap_track_stats_moves_track_counts() {
        let mut m = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
                Arc::new(TrackData::new(0, 2)),
            ],
            ..Default::default()
        };
        m.load_track_notes(vec![
            vec![note(0, 100, 60), note(200, 300, 60)],
            vec![note(0, 100, 61)],
            vec![note(0, 100, 60)],
        ]);
        m.rebuild();
        assert_eq!(m.track_note_count, vec![2, 1, 1]);
        assert_eq!(m.tick_length, 300);

        // 旧 0→新 1、旧 1→新 2、旧 2→新 0
        let remap = [1u16, 2, 0];
        m.apply_track_remap(&remap);
        m.remap_track_stats(&remap);

        assert_eq!(m.track_note_count, vec![1, 2, 1], "统计应随轨号搬移");
        assert_eq!(m.track_audible_count, vec![1, 2, 1]);
        assert_eq!(m.note_count, 4, "音符总数不变");
        assert_eq!(m.tick_length, 300, "tick_length 不变");
        assert_eq!(m.bucket_note_count[60], 3, "桶计数不变");
        assert_eq!(m.bucket_track_stats[60].get(&1), Some(&(2, 2)));
        assert_eq!(m.bucket_track_stats[60].get(&0), Some(&(1, 1)));
        assert_eq!(m.bucket_track_stats[61].get(&2), Some(&(1, 1)));
        assert_eq!(m.bucket_track_stats[61].get(&1), None, "旧键应消失");
    }

    /// remap_track_stats：中间插入新轨时，新轨槽位统计为 0（max 新轨号 + 1）。
    #[test]
    fn remap_track_stats_insert_leaves_new_slot_empty() {
        let mut m = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            ..Default::default()
        };
        m.load_track_notes(vec![vec![note(0, 100, 60)], vec![note(0, 100, 61)]]);
        m.rebuild();

        // 在 idx 1 插入新轨：旧 0→0、旧 1→2
        m.tracks.insert(1, Arc::new(TrackData::new(0, 2)));
        let remap = [0u16, 2];
        m.apply_track_remap(&remap);
        m.remap_track_stats(&remap);

        assert_eq!(m.track_note_count, vec![1, 0, 1], "新轨槽位为 0");
        assert_eq!(m.bucket_track_stats[60].get(&0), Some(&(1, 1)));
        assert_eq!(m.bucket_track_stats[61].get(&2), Some(&(1, 1)));
        assert_eq!(m.bucket_track_stats[61].get(&1), None);
    }

    /// push_empty_track_stats：末尾追加空轨槽位（旧统计不变）。
    #[test]
    fn push_empty_track_stats_appends_zero_slots() {
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(vec![vec![note(0, 100, 60)]]);
        m.rebuild();
        assert_eq!(m.track_note_count, vec![1]);

        m.tracks.push(Arc::new(TrackData::new(0, 1)));
        m.tracks.push(Arc::new(TrackData::new(0, 2)));
        m.push_empty_track_stats(2);

        assert_eq!(m.track_note_count, vec![1, 0, 0]);
        assert_eq!(m.track_audible_count, vec![1, 0, 0]);
        assert_eq!(m.note_count, 1);
        assert_eq!(m.tick_length, 100);
    }

    /// apply_track_remap：恒等 remap 是 no-op（不触碰音符）。
    #[test]
    fn apply_track_remap_identity_is_noop() {
        let mut m = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            ..Default::default()
        };
        m.load_track_notes(vec![vec![note(0, 100, 60)], vec![note(0, 100, 61)]]);
        m.rebuild();
        let before_ids: Vec<u32> = m.notes[60].iter().map(|n| n.id).collect();

        m.apply_track_remap(&[0, 1]);

        assert_eq!(m.notes[60][0].track, 0);
        assert_eq!(m.notes[61][0].track, 1);
        let after_ids: Vec<u32> = m.notes[60].iter().map(|n| n.id).collect();
        assert_eq!(before_ids, after_ids);
    }

    /// apply_track_remap：删除轨（u16::MAX）与重编号一趟完成，越界按删除。
    #[test]
    fn apply_track_remap_deletes_and_shifts() {
        let mut m = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
                Arc::new(TrackData::new(0, 2)),
            ],
            ..Default::default()
        };
        m.load_track_notes(vec![
            vec![note(0, 480, 60)],
            vec![note(0, 480, 62)],
            vec![note(0, 480, 64)],
        ]);
        m.rebuild();
        // 删 track0，track1→0，track2→1。
        m.apply_track_remap(&[u16::MAX, 0, 1]);
        m.rebuild();
        assert_eq!(m.notes[60].len(), 0, "被删轨的音符应移除");
        assert_eq!(m.notes[62][0].track, 0);
        assert_eq!(m.notes[64][0].track, 1);
        assert_eq!(m.track_note_count, vec![1, 1, 0]);
        assert!(m.notes.iter().all(|b| b.is_sorted()));
    }

    /// ensure_conductor_track：非 conductor 头部 → 插入并全轨号 +1；幂等。
    #[test]
    fn ensure_conductor_track_inserts_and_shifts() {
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(vec![vec![note(0, 480, 60)]]);
        m.rebuild();
        assert!(!m.has_conductor_track(), "头部有音符 → 不是 conductor");
        assert!(m.ensure_conductor_track());
        m.rebuild();
        assert!(m.has_conductor_track());
        assert_eq!(m.tracks.len(), 2);
        assert_eq!(m.tracks[0].name, "Conductor");
        assert_eq!(m.notes[60][0].track, 1, "原音符轨号 +1");
        assert!(!m.ensure_conductor_track(), "幂等：已有 conductor 不再插入");

        let mut empty = YinModel::default();
        assert!(!empty.ensure_conductor_track(), "空模型不插入");
    }

    /// Master 角色：旧存档（头部 Midi 但空）加载时补齐为 `TrackKind::Master`；
    /// 一旦标记，身份不再依赖内容（即使混入音符仍是 conductor）。
    #[test]
    fn conductor_role_is_marked_and_survives_notes() {
        // 新建工程：conductor 自带 Master。
        let fresh = YinModel::new_empty_with_16_tracks();
        assert_eq!(fresh.tracks[0].kind, TrackKind::Master);

        // 旧存档：头部是 Midi 但为空 → 启发式识别，并补齐 Master 标记。
        let mut legacy = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            ..Default::default()
        };
        legacy.rebuild();
        assert!(legacy.has_conductor_track(), "旧存档启发式识别 conductor");
        assert!(!legacy.ensure_conductor_track(), "已有 conductor 不再插入");
        assert_eq!(
            legacy.tracks[0].kind,
            TrackKind::Master,
            "应补齐 Master 角色标记"
        );

        // 一旦标记 Master：即使 0 号轨混入音符，身份也不丢。
        let mut polluted = YinModel::new_empty_with_16_tracks();
        polluted.load_track_notes(vec![vec![note(0, 0, 60)]]);
        polluted.rebuild();
        assert!(
            polluted.has_conductor_track(),
            "Master 身份不依赖内容，混入音符仍是 conductor"
        );
    }

    #[test]
    fn note_loader_fills_buckets_and_assigns_ids() {
        // 流式加载路径（.yin v9）：逐音符喂 NoteLoader；
        // id=0 重新分配，非 0 保留。
        let mut counts = [0u32; KEY_COUNT];
        counts[0] = 1;
        counts[1] = 2;
        let mut loader = NoteLoader::new(2, 1, counts);
        loader.feed(0, 1, 0, 480, 100, 0);
        loader.feed(1, 0, 960, 1440, 90, 9);
        loader.feed(1, 1, 960, 1920, 80, 0);

        let mut m = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            ..Default::default()
        };
        loader.finish(&mut m);

        assert_eq!(m.note_count, 3);
        assert_eq!(m.notes[0][0].track, 1);
        assert_eq!(m.notes[0][0].start_tick, 0);
        assert_eq!(m.notes[1].len(), 2);
        assert_eq!(m.notes[1][0].track, 0);
        assert_eq!(m.notes[1][1].track, 1);
        // id=0 的从 1 开始分配，不与已保留的 id=9 冲突
        let mut ids: Vec<u32> = m
            .notes
            .iter()
            .flat_map(|b| b.iter().map(|n| n.id))
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2, 9]);
        assert_eq!(m.track_note_count[1], 2);
        assert_eq!(m.next_note_id, 10);
    }

    #[test]
    fn rebuild_counts_and_buckets_notes() {
        let per_track = vec![vec![
            note(0, 480, 60),
            note(480, 960, 64),
            note(960, 1920, 60),
        ]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        assert_eq!(m.note_count, 3);
        assert_eq!(m.tick_length, 1920);
        assert_eq!(m.notes[60].len(), 2);
        assert_eq!(m.notes[64].len(), 1);
        assert_eq!(m.notes[60][0].start_tick, 0);
        assert_eq!(m.notes[60][1].start_tick, 960);
    }

    #[test]
    fn rebuild_sorts_per_key() {
        // Insert notes out-of-order; cache should be sorted by start_tick.
        let per_track = vec![vec![note(960, 1920, 60), note(0, 480, 60)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        assert_eq!(m.notes[60][0].start_tick, 0);
        assert_eq!(m.notes[60][1].start_tick, 960);
    }

    #[test]
    fn rebuild_builds_tempo_map_with_default_when_no_tempo() {
        let mut m = YinModel::default();
        m.rebuild();
        // Default tempo segment at tick 0 with 120 BPM.
        assert_eq!(m.tempo_map.tempo_segments.len(), 1);
        assert!((m.tempo_map.tempo_segments[0].start_time - 0.0).abs() < 1e-9);
    }

    #[test]
    fn rebuild_builds_tempo_map_from_conductor() {
        let mut conductor = ConductorData::default();
        conductor.tempo.events.push(yinhe_types::AutomationEvent {
            id: 0,
            tick: 0,
            value: 120.0,
            shape: yinhe_types::SegmentShape::Step,
        });
        conductor.tempo.events.push(yinhe_types::AutomationEvent {
            id: 0,
            tick: 1920,
            value: 60.0,
            shape: yinhe_types::SegmentShape::Step,
        });

        let mut m = YinModel {
            conductor: Arc::new(conductor),
            ..Default::default()
        };
        m.rebuild();
        assert_eq!(m.tempo_map.tempo_segments.len(), 2);
        // Second segment at tick 1920, time = 1920/480 * 0.5s/quarter = 2s
        let expected = 1920.0 / 480.0 * 0.5;
        assert!((m.tempo_map.tempo_segments[1].start_time - expected).abs() < 1e-6);
    }

    #[test]
    fn rebuild_inserts_implicit_zero_tempo_segment() {
        // Tempo events that don't start at tick 0 should still produce a
        // segment at tick 0 (default 120 BPM) so lookups before the first
        // tempo find something.
        let mut conductor = ConductorData::default();
        conductor.tempo.events.push(yinhe_types::AutomationEvent {
            id: 0,
            tick: 1920,
            value: 60.0,
            shape: yinhe_types::SegmentShape::Step,
        });

        let mut m = YinModel {
            conductor: Arc::new(conductor),
            ..Default::default()
        };
        m.rebuild();
        assert_eq!(m.tempo_map.tempo_segments.len(), 2);
        assert_eq!(m.tempo_map.tempo_segments[0].start_tick, 0);
    }

    fn set_tempo_events(conductor: &mut ConductorData, events: &[(u32, f32)]) {
        for &(tick, value) in events {
            conductor.tempo.events.push(yinhe_types::AutomationEvent {
                id: 0,
                tick,
                value,
                shape: yinhe_types::SegmentShape::Step,
            });
        }
    }

    #[test]
    fn zero_bpm_jumps_to_next_tempo_event() {
        let mut conductor = ConductorData::default();
        set_tempo_events(&mut conductor, &[(0, 120.0), (480, 0.0), (960, 60.0)]);
        let mut m = YinModel {
            conductor: Arc::new(conductor),
            tick_length: 1920,
            ..Default::default()
        };
        m.rebuild_tempo_map();
        let tm = &m.tempo_map;

        assert_eq!(tm.skip_ranges, vec![(480, 960)]);
        // 剪切区间内时间恒定，等于 480 tick 处（0.5s）。
        assert!((tm.tick_to_seconds(480) - 0.5).abs() < 1e-9);
        assert!((tm.tick_to_seconds(700) - 0.5).abs() < 1e-9);
        assert!((tm.tick_to_seconds(960) - 0.5).abs() < 1e-9);
        // 960 起 60 BPM：480 tick = 1s。
        assert!((tm.tick_to_seconds(1440) - 1.5).abs() < 1e-9);
        // 时间 0.499s 还在剪切前；0.5s 直接跳到 960。
        assert!((tm.tick_at_time(0.499) - 479.04).abs() < 0.1);
        assert!((tm.tick_at_time(0.5) - 960.0).abs() < 1e-6);
        assert!((tm.tick_at_time(1.0) - 1200.0).abs() < 1e-6);
        // 剪切段内 BPM 显示 0；目标处恢复 60。
        assert_eq!(tm.bpm_at_tick(700), 0.0);
        assert!((tm.bpm_at_tick(960) - 60.0).abs() < 0.01);

        // 音频片段（秒）：剪切段在原始时间轴上占 0.5s..1.0s。
        assert_eq!(tm.skip_time_ranges.len(), 1);
        assert!((tm.skip_time_ranges[0].0 - 0.5).abs() < 1e-9);
        assert!((tm.skip_time_ranges[0].1 - 1.0).abs() < 1e-9);
        // 跨剪切起点 → 截断。
        let (s, e) = tm.cut_clip_times(0.2, 2.0).unwrap();
        assert!((s - 0.2).abs() < 1e-9);
        assert!((e - 0.5).abs() < 1e-9);
        // 剪切之后 → 左移 0.5s。
        let (s, e) = tm.cut_clip_times(1.2, 2.0).unwrap();
        assert!((s - 0.7).abs() < 1e-9);
        assert!((e - 1.5).abs() < 1e-9);
        // 剪切段内 → 丢弃。
        assert!(tm.cut_clip_times(0.6, 0.9).is_none());
        // 正好结束在剪切起点 → 保留。
        let (s, e) = tm.cut_clip_times(0.4, 0.5).unwrap();
        assert!((s - 0.4).abs() < 1e-9);
        assert!((e - 0.5).abs() < 1e-9);
    }

    #[test]
    fn zero_bpm_skips_consecutive_zeros() {
        let mut conductor = ConductorData::default();
        set_tempo_events(
            &mut conductor,
            &[(0, 120.0), (480, 0.0), (960, 0.0), (1440, 60.0)],
        );
        let mut m = YinModel {
            conductor: Arc::new(conductor),
            tick_length: 1920,
            ..Default::default()
        };
        m.rebuild_tempo_map();
        let tm = &m.tempo_map;

        // 两个 0 合并为一段 [480, 1440)，直接跳到 1440。
        assert_eq!(tm.skip_ranges, vec![(480, 1440)]);
        assert!((tm.tick_to_seconds(700) - 0.5).abs() < 1e-9);
        assert!((tm.tick_to_seconds(1440) - 0.5).abs() < 1e-9);
        assert!((tm.tick_at_time(0.5) - 1440.0).abs() < 1e-6);
    }

    #[test]
    fn zero_bpm_at_start_jumps_to_first_valid_tempo() {
        let mut conductor = ConductorData::default();
        set_tempo_events(&mut conductor, &[(0, 0.0), (960, 60.0)]);
        let mut m = YinModel {
            conductor: Arc::new(conductor),
            tick_length: 1920,
            ..Default::default()
        };
        m.rebuild_tempo_map();
        let tm = &m.tempo_map;

        assert_eq!(tm.skip_ranges, vec![(0, 960)]);
        assert!((tm.tick_to_seconds(0) - 0.0).abs() < 1e-9);
        assert!((tm.tick_to_seconds(500) - 0.0).abs() < 1e-9);
        assert!((tm.tick_to_seconds(960) - 0.0).abs() < 1e-9);
        assert!((tm.tick_to_seconds(1440) - 1.0).abs() < 1e-9);
        assert!((tm.tick_at_time(0.0) - 960.0).abs() < 1e-6);
    }

    #[test]
    fn trailing_zero_bpm_jumps_to_song_end() {
        let mut conductor = ConductorData::default();
        set_tempo_events(&mut conductor, &[(0, 120.0), (960, 0.0)]);
        let mut m = YinModel {
            conductor: Arc::new(conductor),
            tick_length: 1920,
            ..Default::default()
        };
        m.rebuild_tempo_map();
        let tm = &m.tempo_map;

        assert_eq!(tm.skip_ranges, vec![(960, 1920)]);
        assert!((tm.tick_to_seconds(960) - 1.0).abs() < 1e-9);
        assert!((tm.tick_to_seconds(1920) - 1.0).abs() < 1e-9);
        assert!((tm.duration_seconds() - 1.0).abs() < 1e-9);
        assert!((tm.tick_at_time(1.0) - 1920.0).abs() < 1e-6);
    }

    #[test]
    fn zero_bpm_playable_note_end_clamps_crossing_notes() {
        let mut conductor = ConductorData::default();
        set_tempo_events(&mut conductor, &[(0, 120.0), (480, 0.0), (960, 60.0)]);
        let mut m = YinModel {
            conductor: Arc::new(conductor),
            tick_length: 1920,
            ..Default::default()
        };
        m.rebuild_tempo_map();
        let tm = &m.tempo_map;

        // 在剪切段内开始 → 不发声。
        assert_eq!(tm.playable_note_end(600, 700), None);
        // 跨过剪切起点 → 在 480 截断。
        assert_eq!(tm.playable_note_end(0, 1000), Some(480));
        // 结束正好在剪切起点 → 保持。
        assert_eq!(tm.playable_note_end(0, 480), Some(480));
        // 剪切之后 → 原样。
        assert_eq!(tm.playable_note_end(960, 1440), Some(1440));
    }

    #[test]
    fn track_global_channel() {
        let t = TrackData::new(2, 5);
        // (port=2 << 4) | (channel=5) = 0x25
        assert_eq!(t.global_channel(), 0x25);
    }

    #[test]
    fn track_uuid_is_unique() {
        let a = TrackData::new(0, 0);
        let b = TrackData::new(0, 0);
        assert_ne!(a.uuid, b.uuid);
    }

    /// Verify `rebuild_dirty` keeps `track_note_count` / `track_audible_count`
    /// consistent when notes are added/removed/edited. We compare the
    /// incremental update path against a full `rebuild()` from scratch.
    fn note_audible(start: u32, end: u32, key: u8) -> NoteEvent {
        NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: end,
            key,
            velocity: 100,
        }
    }

    fn note_silent(start: u32, end: u32, key: u8) -> NoteEvent {
        NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: end,
            key,
            velocity: 0, // silent — must not count toward `track_audible_count`
        }
    }

    #[test]
    fn rebuild_dirty_keeps_track_stats_consistent() {
        // Build a model with 2 tracks and 4 notes, then mutate it and
        // compare `rebuild_dirty`'s incremental track stats against a
        // full `rebuild()` of the same final state.
        let per_track = vec![
            vec![note_audible(0, 480, 60), note_audible(480, 960, 64)],
            vec![note_silent(0, 480, 60), note_audible(0, 240, 62)],
        ];
        let mut base = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            ..Default::default()
        };
        base.load_track_notes(per_track);

        // Final desired state: same notes as `base` plus one new silent
        // note in bucket 60 (track 1) and one new audible note in
        // bucket 62 (track 1). We compute this in two ways and compare.
        let mut m_inc = base.clone();
        {
            let model = Arc::make_mut(&mut m_inc.notes[60]);
            model.insert_sorted(yinhe_types::Note {
                id: 0,
                start_tick: 960,
                end_tick: 1440,
                velocity: 0,
                track: 1,
            });
        }
        {
            let model = Arc::make_mut(&mut m_inc.notes[62]);
            model.insert_sorted(yinhe_types::Note {
                id: 0,
                start_tick: 240,
                end_tick: 480,
                velocity: 80,
                track: 1,
            });
        }
        m_inc.mark_dirty(60);
        m_inc.mark_dirty(62);
        m_inc.rebuild_dirty();

        let mut m_full = base.clone();
        {
            let model = Arc::make_mut(&mut m_full.notes[60]);
            model.insert_sorted(yinhe_types::Note {
                id: 0,
                start_tick: 960,
                end_tick: 1440,
                velocity: 0,
                track: 1,
            });
        }
        {
            let model = Arc::make_mut(&mut m_full.notes[62]);
            model.insert_sorted(yinhe_types::Note {
                id: 0,
                start_tick: 240,
                end_tick: 480,
                velocity: 80,
                track: 1,
            });
        }
        m_full.rebuild();

        assert_eq!(
            m_inc.track_note_count, m_full.track_note_count,
            "track_note_count drift"
        );
        assert_eq!(
            m_inc.track_audible_count, m_full.track_audible_count,
            "track_audible_count drift"
        );
        assert_eq!(m_inc.note_count, m_full.note_count);
        assert_eq!(m_inc.tick_length, m_full.tick_length);

        // Sanity: track 0 = 2 audible, 0 silent. track 1 = 2 audible, 2 silent.
        assert_eq!(m_full.track_note_count, vec![2, 4]);
        assert_eq!(m_full.track_audible_count, vec![2, 2]);

        // Now test removal: remove a note from bucket 60 in track 0.
        let mut m_del = m_full.clone();
        {
            let model = Arc::make_mut(&mut m_del.notes[60]);
            model.retain(|n| !(n.track == 0 && n.start_tick == 0));
        }
        m_del.mark_dirty(60);
        m_del.rebuild_dirty();

        let mut m_del_ref = m_full.clone();
        {
            let model = Arc::make_mut(&mut m_del_ref.notes[60]);
            model.retain(|n| !(n.track == 0 && n.start_tick == 0));
        }
        m_del_ref.rebuild();

        assert_eq!(
            m_del.track_note_count, m_del_ref.track_note_count,
            "track_note_count drift after remove"
        );
        assert_eq!(
            m_del.track_audible_count, m_del_ref.track_audible_count,
            "track_audible_count drift after remove"
        );
    }

    /// 回归测试：删除全曲最后一个音符（max end_tick 所在音符）后，
    /// `rebuild_dirty` 必须收缩 `tick_length`。
    /// 之前 bug：`new_tick_length` 从 `self.tick_length` 起步只取 max，
    /// 永远不会缩小，导致 select_all_pr/ar 范围、播放结束判定错误。
    #[test]
    fn rebuild_dirty_tick_length_shrinks_after_deleting_last_note() {
        let per_track = vec![
            // track 0: 两个音符，end_tick 分别为 480 和 1920（末尾）
            vec![note_audible(0, 480, 60), note_audible(480, 1920, 62)],
        ];
        let mut m = YinModel {
            tracks: vec![Arc::new(super::TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        assert_eq!(
            m.tick_length, 1920,
            "初始 tick_length 应为末尾音符 end_tick"
        );

        // 删除 end_tick=1920 的末尾音符
        {
            let bucket = Arc::make_mut(&mut m.notes[62]);
            bucket.retain(|n| !(n.start_tick == 480 && n.end_tick == 1920));
        }
        m.mark_dirty(62);
        m.rebuild_dirty();

        // tick_length 必须收缩到 480（剩下的音符 end_tick）
        assert_eq!(
            m.tick_length, 480,
            "删除末尾音符后 tick_length 必须收缩（bug C-3 回归）"
        );

        // 再删掉剩下的音符 → tick_length 应为 0
        {
            let bucket = Arc::make_mut(&mut m.notes[60]);
            bucket.clear();
        }
        m.mark_dirty(60);
        m.rebuild_dirty();
        assert_eq!(m.tick_length, 0, "清空所有音符后 tick_length 必须为 0");
    }

    // -----------------------------------------------------------------------
    // rescale_ppq 测试
    // -----------------------------------------------------------------------

    #[test]
    fn rescale_ppq_doubles_ticks_for_integer_ratio() {
        // 480 → 960：所有 tick ×2，整数比例，精确。
        let per_track = vec![vec![note(0, 480, 60), note(480, 960, 64)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        assert_eq!(m.meta.ppq, 480);

        m.rescale_ppq(960);
        assert_eq!(m.meta.ppq, 960);
        assert_eq!(m.notes[60][0].start_tick, 0);
        assert_eq!(m.notes[60][0].end_tick, 960);
        assert_eq!(m.notes[64][0].start_tick, 960);
        assert_eq!(m.notes[64][0].end_tick, 1920);
        assert_eq!(m.tick_length, 1920);
        assert_eq!(m.tempo_map.ticks_per_beat, 960);
    }

    #[test]
    fn rescale_ppq_halves_ticks() {
        // 960 → 480：所有 tick ÷2，整数比例，精确。
        let per_track = vec![vec![note(0, 960, 60), note(960, 1920, 64)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            meta: ProjectMeta {
                ppq: 960,
                ..Default::default()
            },
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();

        m.rescale_ppq(480);
        assert_eq!(m.meta.ppq, 480);
        assert_eq!(m.notes[60][0].start_tick, 0);
        assert_eq!(m.notes[60][0].end_tick, 480);
        assert_eq!(m.notes[64][0].start_tick, 480);
        assert_eq!(m.notes[64][0].end_tick, 960);
    }

    #[test]
    fn rescale_ppq_scales_automation_and_conductor() {
        // 验证 conductor.tempo / time_sig / track automation / pc 的 tick 也被缩放。
        use yinhe_types::{
            AutomationEvent, AutomationLane, AutomationTarget, PcEvent, SegmentShape, TimeSigEvent,
        };
        let mut conductor = ConductorData::default();
        conductor.tempo.events.push(AutomationEvent {
            id: 0,
            tick: 480,
            value: 120.0,
            shape: SegmentShape::Step,
        });
        conductor.time_sig.push(TimeSigEvent {
            tick: 960,
            numerator: 4,
            denominator: 2,
        });

        let mut t0 = TrackData::new(0, 0);
        t0.automation_lanes = vec![AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
                tick: 240,
                value: 100.0,
                shape: SegmentShape::Step,
            }],
        }];
        t0.program_change = vec![PcEvent {
            tick: 120,
            program: 5,
            bank_msb: 0,
            bank_lsb: 0,
        }];

        let mut m = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t0)],
            ..Default::default()
        };
        m.rebuild();

        m.rescale_ppq(960);
        // 480→960: ×2
        assert_eq!(m.conductor.tempo.events[0].tick, 960);
        assert_eq!(m.conductor.time_sig[0].tick, 1920);
        assert_eq!(m.tracks[0].automation_lanes[0].events[0].tick, 480);
        assert_eq!(m.tracks[0].program_change[0].tick, 240);
    }

    #[test]
    fn rescale_ppq_noop_when_same() {
        let per_track = vec![vec![note(0, 480, 60)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        let before = m.clone();
        m.rescale_ppq(480);
        assert_eq!(m.notes[60][0].start_tick, before.notes[60][0].start_tick);
        assert_eq!(m.notes[60][0].end_tick, before.notes[60][0].end_tick);
    }

    /// 异步 PPQ 缩放也必须覆盖标记/调号/歌词/和弦（旧实现漏了这些 tick）。
    #[test]
    fn rescale_ppq_with_progress_scales_all_event_lists() {
        use yinhe_types::{
            ChordEvent, KeySigEvent, LyricsEvent, MarkerEvent, ScaleType, TimeSigEvent,
        };

        let mut conductor = ConductorData::default();
        conductor.time_sig.push(TimeSigEvent {
            tick: 240,
            numerator: 3,
            denominator: 2,
        });
        conductor.key_sig.push(KeySigEvent {
            tick: 480,
            root: 0,
            scale: ScaleType::Major,
        });
        conductor.markers.push(MarkerEvent {
            tick: 600,
            text: "A".into(),
        });
        conductor.lyrics.push(LyricsEvent {
            tick: 720,
            text: "la".into(),
        });
        conductor.chord.push(ChordEvent {
            tick: 840,
            text: "C".into(),
        });
        let mut t0 = TrackData::new(0, 0);
        t0.lyrics.push(LyricsEvent {
            tick: 960,
            text: "track".into(),
        });
        t0.chord.push(ChordEvent {
            tick: 1200,
            text: "G".into(),
        });

        let m = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t0)],
            ..Default::default()
        };
        let progress = Arc::new(std::sync::Mutex::new(
            crate::model_stats::RescaleProgress::default(),
        ));
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let m =
            YinModel::rescale_ppq_with_progress(m, 960, progress, cancel).expect("rescale 应成功");
        assert_eq!(m.conductor.time_sig[0].tick, 480);
        assert_eq!(m.conductor.key_sig[0].tick, 960);
        assert_eq!(m.conductor.markers[0].tick, 1200);
        assert_eq!(m.conductor.lyrics[0].tick, 1440);
        assert_eq!(m.conductor.chord[0].tick, 1680);
        assert_eq!(m.tracks[0].lyrics[0].tick, 1920);
        assert_eq!(m.tracks[0].chord[0].tick, 2400);
    }

    #[test]
    fn rescale_ppq_round_trip_restores_original() {
        // 480 → 960 → 480：整数比例下 round-trip 精确还原。
        let per_track = vec![vec![note(0, 480, 60), note(480, 960, 64)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        let original = m.clone();

        m.rescale_ppq(960);
        m.rescale_ppq(480);

        assert_eq!(m.meta.ppq, 480);
        assert_eq!(m.notes[60][0].start_tick, original.notes[60][0].start_tick);
        assert_eq!(m.notes[60][0].end_tick, original.notes[60][0].end_tick);
        assert_eq!(m.notes[64][0].start_tick, original.notes[64][0].start_tick);
        assert_eq!(m.notes[64][0].end_tick, original.notes[64][0].end_tick);
    }

    // -----------------------------------------------------------------------
    // new_empty_with_16_tracks 测试
    // -----------------------------------------------------------------------

    #[test]
    fn new_empty_with_16_tracks_creates_conductor_plus_16_tracks() {
        let mut m = YinModel::new_empty_with_16_tracks();
        m.rebuild();
        assert_eq!(m.tracks.len(), 17);
        assert_eq!(m.tracks[0].name, "Conductor");
        assert_eq!(m.tracks[1].name, "Track 1");
        assert_eq!(m.tracks[16].name, "Track 16");
        // Conductor 应在 ch 0
        assert_eq!(m.tracks[0].channel, 0);
        // Track 1 在 ch 0, Track 16 在 ch 15
        assert_eq!(m.tracks[1].channel, 0);
        assert_eq!(m.tracks[16].channel, 15);
        // 新工程不写原生事件：默认 120 BPM + 4/4 由 ProjectMeta 兜底。
        assert!(m.conductor.tempo.events.is_empty());
        assert!(m.conductor.time_sig.is_empty());
        assert_eq!(m.meta.default_bpm, 120.0);
        assert_eq!(m.meta.default_time_sig, (4, 2));
        // tempo_map 应被 rebuild 出来（默认值生效）
        assert_eq!(m.tempo_map.tempo_segments.len(), 1);
        assert_eq!(m.tempo_map.tempo_segments[0].micros_per_quarter, 500_000);
        assert_eq!(m.tempo_map.time_sig_default, (4, 2));
    }

    #[test]
    fn song_info_migrates_legacy_artist_and_copyright_into_artists() {
        let mut meta = ProjectMeta {
            artist: "Legacy".into(),
            copyright: "(c) Label".into(),
            ..Default::default()
        };
        let migrated = SongInfo::from_meta(&meta);
        assert_eq!(migrated.artists.len(), 2, "artist 与 copyright 各自成框");
        assert_eq!(migrated.artists[0].name, "Legacy");
        assert_eq!(migrated.artists[1].name, "(c) Label");
        assert_eq!(migrated.copyright, "Legacy / (c) Label");

        let mut info = SongInfo::from_meta(&meta);
        info.artist = "stale".into();
        info.artists = vec![
            ArtistRef {
                name: "A".into(),
                ..Default::default()
            },
            ArtistRef {
                name: "B".into(),
                ..Default::default()
            },
        ];
        info.apply_to(&mut meta);
        assert_eq!(meta.artists.len(), 2);
        assert_eq!(meta.artist, "A", "兼容字段由首位艺术家同步");
        assert_eq!(meta.copyright, "A / B", "单字段格式写合并文本");

        // 列表为空时保留手填的 artist / copyright。
        info.artists.clear();
        info.artist = "Fallback".into();
        info.copyright = "Keep".into();
        info.apply_to(&mut meta);
        assert_eq!(meta.artist, "Fallback");
        assert_eq!(meta.copyright, "Keep");
    }

    #[test]
    fn new_empty_project_makes_a10_drum() {
        let m = YinModel::new_empty_with_16_tracks();
        assert_eq!(
            m.channel_instruments[9],
            Some(ChannelInstrument {
                bank_msb: 0,
                bank_lsb: 0,
                program: 0,
                is_drum: true,
            }),
            "A10（索引 9）默认 Standard Kit"
        );
        assert_eq!(m.channel_instruments[8], None);
        assert_eq!(m.channel_instruments[10], None);
    }

    /// 旧工程用 `bank_msb == 128` 表示鼓：加载时迁移到显式 `is_drum`，
    /// 并把 bank 归 0（鼓组不占真实 bank 号）。
    #[test]
    fn legacy_drum_bank_migrates_to_explicit_flag() {
        let mut inst = ChannelInstrument {
            bank_msb: 128,
            bank_lsb: 0,
            program: 3,
            is_drum: false,
        };
        inst.normalize_legacy_bank();
        assert!(inst.is_drum);
        assert_eq!((inst.bank_msb, inst.program), (0, 3));

        // 真实 bank（< 128）原样保留，不当鼓。
        let mut melodic = ChannelInstrument {
            bank_msb: 17,
            bank_lsb: 2,
            program: 40,
            is_drum: false,
        };
        melodic.normalize_legacy_bank();
        assert!(!melodic.is_drum);
        assert_eq!(
            (melodic.bank_msb, melodic.bank_lsb, melodic.program),
            (17, 2, 40)
        );
    }

    #[test]
    fn song_info_skips_derived_copyright_mirror() {
        let meta = ProjectMeta {
            artist: "A".into(),
            copyright: "A / B".into(),
            artists: vec![
                ArtistRef {
                    name: "A".into(),
                    ..Default::default()
                },
                ArtistRef {
                    name: "B".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let info = SongInfo::from_meta(&meta);
        assert_eq!(info.artists.len(), 2, "派生合并文本不能再变成艺术家 3");
    }

    #[test]
    fn audio_clip_warp_maps_progress_and_reverse() {
        let mut c = clip(1, 0.0, 2.0, 0.0, 0.0);
        c.offset_seconds = 0.25;
        c.warp = Some(WarpMap::identity(4.0, 0.25, 2.0, true));
        assert!((c.source_seconds_at_progress(0.0, false).unwrap() - 0.25).abs() < 1e-9);
        assert!((c.source_seconds_at_progress(0.5, false).unwrap() - 1.25).abs() < 1e-9);
        assert!((c.source_seconds_at_progress(1.0, false).unwrap() - 2.25).abs() < 1e-9);
        // 反向：进度 0 从素材尾开始。
        assert!((c.source_seconds_at_progress(0.0, true).unwrap() - 2.25).abs() < 1e-9);
        assert!((c.source_seconds_at_progress(1.0, true).unwrap() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn clip_fade_gain_endpoints_and_midpoint() {
        assert!((clip_fade_gain(0.0, 2.0, 0.5, 0.5) - 0.0).abs() < 1e-6);
        assert!((clip_fade_gain(2.0, 2.0, 0.5, 0.5) - 0.0).abs() < 1e-6);
        assert!((clip_fade_gain(1.0, 2.0, 0.5, 0.5) - 1.0).abs() < 1e-6);
        assert!((clip_fade_gain(0.25, 2.0, 0.5, 0.0) - 0.5).abs() < 1e-6);
        // 超长 fade 在位置 0.5 处按 0.5/10 线性，不会越界。
        assert!((clip_fade_gain(0.5, 1.0, 10.0, 0.0) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn audio_clip_follows_tempo_change() {
        let mut m = YinModel::default();
        let mut track = TrackData::new(0, 0);
        let mut c = clip(1, 0.0, 2.0, 0.0, 0.0);
        c.offset_seconds = 0.25;
        track.audio_clips.push(c);
        m.tracks.push(Arc::new(track));
        m.rebuild_tempo_map(); // 120 BPM 默认 → 2s = 4 beats

        let c = &m.tracks[0].audio_clips[0];
        assert!((c.start_beat.unwrap() - 0.0).abs() < 1e-6);
        assert!((c.warp.as_ref().unwrap().beat_span() - 4.0).abs() < 1e-6);
        assert!((c.warp.as_ref().unwrap().source_span() - 2.0).abs() < 1e-6);

        // 240 BPM：同样 4 beats 应缩成 1s，素材跨度不变。
        let conductor = Arc::make_mut(&mut m.conductor);
        set_tempo_events(conductor, &[(0, 240.0)]);
        m.rebuild_tempo_map();
        let c = &m.tracks[0].audio_clips[0];
        assert!((c.start_seconds - 0.0).abs() < 1e-6);
        assert!(
            (c.duration_seconds - 1.0).abs() < 1e-6,
            "{}",
            c.duration_seconds
        );
        assert!((c.warp.as_ref().unwrap().source_span() - 2.0).abs() < 1e-6);

        // 用户移动片段到 2s：beat 锚点按 240 BPM 变成 8 beats。
        let track = Arc::make_mut(&mut m.tracks[0]);
        track.audio_clips[0].start_seconds = 2.0;
        m.sync_audio_clip_starts();
        let c = &m.tracks[0].audio_clips[0];
        assert!(
            (c.start_beat.unwrap() - 8.0).abs() < 1e-6,
            "{:?}",
            c.start_beat
        );
    }
}

/// Display-oriented track info derived from `TrackData` for UI panels.
#[derive(Clone, Debug)]
pub struct TrackInfo {
    pub index: u16,
    pub name: String,
    pub note_count: u64,
    pub event_count: u64,
    pub port: u8,
    pub channel: u8,
}
