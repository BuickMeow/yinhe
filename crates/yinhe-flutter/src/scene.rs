//! 渲染场景：文档快照（模型 + PR/AR 双视口 + 颜色）与演示内容生成。
//!
//! 渲染线程只读快照；触摸手势（`input.rs`）与首帧初始化写视口。
//! 不跨线程借用 `Document`，避免把 editor-core 的内部可变性带进渲染线程。

use std::sync::{Arc, Mutex, OnceLock};

use yinhe_core::{NoteEvent, YinModel};
use yinhe_types::NoteSource;
use yinhe_editor_core::Document;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{ArrangementView, PianoRollView, TimelineViewBase};

/// 视图模式（与 Dart 约定一致：0=AR 走带，1=PR 卷帘）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Arrangement = 0,
    Pianoroll = 1,
}

/// AR 轨道面板宽度（逻辑像素，与旧安卓壳 PANEL_W 一致）。
pub const AR_TRACK_PANEL_W: f32 = 168.0;
/// AR 行高（逻辑像素，触屏友好的旧壳值）。
pub const AR_LANE_HEIGHT: f32 = 56.0;

/// 轨道面板用的精简轨道信息（Dart 浮层绘制）。
pub struct TrackInfoLite {
    pub name: String,
    pub port: u8,
    pub channel: u8,
    pub color: [f32; 4],
    pub muted: bool,
    pub soloed: bool,
}

/// 渲染线程与触摸手势共享的场景数据。
pub struct RenderScene {
    /// 当前文档模型（None = 无文档，渲染清屏）。
    pub model: Option<Arc<YinModel>>,
    /// PR 视口（逻辑点坐标系）。
    pub pr_view: PianoRollView,
    /// AR 视口。
    pub arr_view: ArrangementView,
    /// 当前视图。
    pub view_mode: ViewMode,
    /// 轨道颜色（RGBA per track）。
    pub track_colors: Vec<[f32; 4]>,
    /// 可见轨（演示阶段统一用 `EditState::pr_visible()` 的超集）。
    pub track_visible: Vec<bool>,
    /// 总 tick 数（视口 clamp 用）。
    pub total_ticks: f64,
    /// 轨道数（AR 布局/vendor 层构建用）。
    pub num_tracks: usize,
    /// 轨道面板信息（名称/端口/颜色/M/S）。
    pub tracks: Vec<TrackInfoLite>,
    /// 每拍 tick 数（网格对齐用）。
    pub ppq: u32,
    /// 文档本体（编辑操作：轨道排序等）。
    pub document: Option<Document>,
}

impl Default for RenderScene {
    fn default() -> Self {
        Self {
            model: None,
            pr_view: PianoRollView::default(),
            arr_view: ArrangementView::default(),
            view_mode: ViewMode::Pianoroll,
            track_colors: Vec::new(),
            track_visible: Vec::new(),
            total_ticks: 0.0,
            num_tracks: 0,
            tracks: Vec::new(),
            ppq: 480,
            document: None,
        }
    }
}

static SCENE: OnceLock<Mutex<RenderScene>> = OnceLock::new();

/// 全局场景（渲染线程每帧取快照；触摸手势直接写视口）。
pub fn scene() -> &'static Mutex<RenderScene> {
    SCENE.get_or_init(|| Mutex::new(RenderScene::default()))
}

/// 生成演示文档并替换当前场景（无文件时的内容源）。
pub fn load_demo() {
    let mut doc = Document::empty();
    // 黑乐谱演示内容必然含重叠音符。
    doc.edit.allow_overlapping_notes = true;
    let gen_max = add_demo_notes(&mut doc);

    // 轨道颜色/显隐（conductor 色用中性浅灰，与主题无关的演示值）。
    doc.sync_track_caches_with_conductor_color([0.85, 0.85, 0.88, 1.0]);

    apply_document(doc, f64::from(gen_max));
}

/// 后台加载用户选择的 MIDI 文件并替换当前场景。
pub fn open_midi(path: String) {
    let spawn = std::thread::Builder::new()
        .name("yinhe-midi-load".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || match load_document(&path) {
            Ok(doc) => apply_document(doc, 0.0),
            Err(e) => log::error!("MIDI 加载失败（{path}）: {e}"),
        });
    if let Err(e) = spawn {
        log::error!("加载线程创建失败: {e}");
    }
}

/// 解析 MIDI 文件并组装为可渲染/可播放的文档。
fn load_document(path: &str) -> Result<Document, String> {
    let started = std::time::Instant::now();
    let model = yinhe_midi::parse_path(path).map_err(|e| format!("解析失败: {e}"))?;
    log::info!(
        "MIDI 解析完成: {} 音符，用时 {:?}",
        model.note_count,
        started.elapsed()
    );

    let project_file = yinhe_yin::ProjectFile::from_meta(&model.meta);
    let mapping_file = yinhe_yin::MappingFile::default();
    let mut doc = Document::from_model(
        path,
        model,
        QuantizePreset::Fraction(1, 4),
        QuantizePreset::Fraction(1, 16),
        project_file,
        mapping_file,
        None,
    )?;
    doc.mark_loaded();
    // 与演示文档一致的 conductor 色（保证两台机器/两种来源配色相同）。
    doc.sync_track_caches_with_conductor_color([0.85, 0.85, 0.88, 1.0]);
    push_recent(path);
    Ok(doc)
}

/// 把文档写入场景快照（演示与文件加载共用）。
///
/// `min_ticks`：演示内容生成时模型 tick_length 尚未刷新，用它兜底总长。
fn apply_document(doc: Document, min_ticks: f64) {
    let total_ticks = (doc.data.model.tick_length as f64).max(min_ticks).max(1.0);

    let mut pr_view = PianoRollView::default();
    // 初始缩放：全曲约 2 屏宽（沿用旧安卓壳 set_model 的算法）。
    pr_view.base.pixels_per_tick = (1280.0 * 2.0 / total_ticks as f32).clamp(0.001, 10.0);

    let arr_base = TimelineViewBase {
        pixels_per_tick: (1280.0 * 2.0 / total_ticks as f32).clamp(0.0005, 8.0),
        scroll_x: 0.0,
        scroll_y: 0.0,
        left_panel_width: AR_TRACK_PANEL_W,
        dirty: true,
        track_panel_row_height: AR_LANE_HEIGHT,
        track_panel_scroll_y: 0.0,
        follow_target: None,
        follow_anim_start: 0.0,
        follow_anim_elapsed: 0.0,
    };
    let arr_view = ArrangementView::with_base(arr_base);

    let model = doc.data.model.clone();
    let note_count = model.note_count;

    let Ok(mut s) = scene().lock() else {
        return;
    };
    s.document = Some(doc);
    s.pr_view = pr_view;
    s.arr_view = arr_view;
    s.view_mode = ViewMode::Arrangement;
    s.total_ticks = total_ticks;
    refresh_snapshot(&mut s);
    let num_tracks = s.num_tracks;
    drop(s);

    log::info!("文档就绪: {note_count} 音符, {total_ticks:.0} tick, {num_tracks} 轨");
    crate::audio::on_model_loaded(model);
}

/// 从文档重算轨道快照（模型/颜色/显隐/轨道面板信息/ppq）。
fn refresh_snapshot(s: &mut RenderScene) {
    let Some(doc) = s.document.as_ref() else {
        return;
    };
    let model = doc.data.model.clone();
    let colors = doc.edit.track_cache.colors.clone();
    s.model = Some(model.clone());
    s.track_colors = colors.clone();
    s.track_visible = doc.edit.pr_visible();
    s.num_tracks = model.tracks.len();
    s.ppq = model.meta.ppq;
    s.tracks = model
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| TrackInfoLite {
            name: t.name.to_string(),
            port: t.port,
            channel: t.channel,
            color: colors.get(i).copied().unwrap_or([0.5, 0.5, 0.5, 1.0]),
            muted: doc
                .edit
                .track_overrides
                .get(i)
                .map(|o| o.muted)
                .unwrap_or(false),
            soloed: doc
                .edit
                .track_overrides
                .get(i)
                .map(|o| o.soloed)
                .unwrap_or(false),
        })
        .collect();
}

/// AR 长按拖动排序：把 `from` 轨移动到 `to` 位置。成功返回 1。
pub fn move_track(from: usize, to: usize) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    {
        let Some(doc) = s.document.as_mut() else {
            return 0;
        };
        if doc.move_track(from, to).is_none() {
            return 0;
        }
    }
    refresh_snapshot(&mut s);
    let model = s.model.clone();
    drop(s);
    // 音轨顺序变化后重载音频（demo 阶段整模型重载）。
    if let Some(model) = model {
        crate::audio::on_model_loaded(model);
    }
    log::info!("轨道已移动: {from} -> {to}");
    1
}

/// 播放光标在当前视图下的 x（逻辑像素；-1 = 无文档）。
///
/// 坐标原点与画布一致（含键盘列/轨道面板偏移），Dart 直接画竖线。
pub fn playhead_x() -> f32 {
    let tick = crate::audio::playback_tick();
    let Ok(s) = scene().lock() else {
        return -1.0;
    };
    if s.model.is_none() {
        return -1.0;
    }
    match s.view_mode {
        ViewMode::Pianoroll => s.pr_view.tick_to_x(tick),
        ViewMode::Arrangement => s.arr_view.base.tick_to_x(tick),
    }
}

/// 生成演示音符（确定性伪随机 + 手工乐段），返回生成内容的最大 tick。
fn add_demo_notes(doc: &mut Document) -> u32 {
    let ppq = doc.data.model.meta.ppq.max(1);
    // 无 rand 依赖的确定性 LCG：唯一样本可复现。
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };

    let bar = ppq * 4;
    let mut max_tick = 0u32;
    // 轨道 1..=6（0 号是 Conductor，add_note 会拒绝）。
    let mut add = |key: u8, t: u32, len: u32, vel: u8, track: u16, max: &mut u32| {
        let _ = doc.add_note(
            track,
            NoteEvent {
                id: 0,
                key,
                start_tick: t,
                end_tick: t + len,
                velocity: vel,
            },
        );
        *max = (*max).max(t + len);
    };

    // 琶音旋律：不同音域、长度/力度有变化。
    for track in 1..=6u16 {
        let base_key = 36 + (track - 1) as u8 * 8;
        let mut t = track as u32 * ppq / 2;
        for i in 0..400u32 {
            let key = base_key + (i % 16) as u8;
            let len = ppq / 4 * (1 + rnd() % 3);
            let vel = (72 + rnd() % 56) as u8;
            add(key, t, len, vel, track, &mut max_tick);
            t += ppq / 4 + rnd() % (ppq / 2);
        }
    }

    // 黑乐谱密度段：每 1/16 拍一排 60 键的柱状簇。
    let cluster_start = bar * 64;
    for i in 0..3600u32 {
        let key = 30 + (i % 60) as u8;
        let t = cluster_start + (i / 60) * (ppq / 16);
        add(key, t, ppq / 8, 80, (i % 6) as u16 + 1, &mut max_tick);
    }

    // 高音区长音铺底。
    for i in 0..16u32 {
        let t = bar * (i + 1);
        add(84, t, bar, 64, 7, &mut max_tick);
    }

    max_tick
}

// ── 最近文件（主页列表，持久化到私有目录 recent.txt）──

struct RecentEntry {
    path: String,
    opened: u64,
}

static RECENT: OnceLock<Mutex<Vec<RecentEntry>>> = OnceLock::new();
static FILES_DIR: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn recent_slot() -> &'static Mutex<Vec<RecentEntry>> {
    RECENT.get_or_init(|| Mutex::new(Vec::new()))
}

fn files_dir_slot() -> &'static Mutex<Option<String>> {
    FILES_DIR.get_or_init(|| Mutex::new(None))
}

/// Kotlin 侧传入私有目录（用于最近文件持久化）。
pub fn set_files_dir(dir: String) {
    if let Ok(mut g) = files_dir_slot().lock() {
        *g = Some(dir);
    }
    load_recent();
}

fn recent_file_path() -> Option<String> {
    files_dir_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|d| format!("{d}/recent.txt")))
}

fn load_recent() {
    let Some(path) = recent_file_path() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let mut list = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(2, '\t');
        let (Some(p), Some(t)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(opened) = t.trim().parse::<u64>() else {
            continue;
        };
        if !p.is_empty() {
            list.push(RecentEntry {
                path: p.to_string(),
                opened,
            });
        }
    }
    list.truncate(10);
    if let Ok(mut slot) = recent_slot().lock() {
        *slot = list;
    }
}

fn save_recent() {
    let Some(path) = recent_file_path() else {
        return;
    };
    let Ok(list) = recent_slot().lock() else {
        return;
    };
    let mut text = String::new();
    for e in list.iter() {
        text.push_str(&e.path);
        text.push('\t');
        text.push_str(&e.opened.to_string());
        text.push('\n');
    }
    if let Err(e) = std::fs::write(&path, text) {
        log::error!("最近文件写入失败: {e}");
    }
}

/// 记录一次成功打开（去重、置顶、限 10 条、持久化）。
fn push_recent(path: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut list) = recent_slot().lock() {
        list.retain(|e| e.path != path);
        list.insert(
            0,
            RecentEntry {
                path: path.to_string(),
                opened: now,
            },
        );
        list.truncate(10);
    }
    save_recent();
}

/// 最近文件 JSON（Dart 解析）：`[{"name","path","time"}]`。
pub fn recent_json() -> String {
    let Ok(list) = recent_slot().lock() else {
        return "[]".into();
    };
    let mut out = String::from("[");
    for (i, e) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let name = e.path.rsplit('/').next().unwrap_or(&e.path);
        out.push_str(&format!(
            "{{\"name\":\"{}\",\"path\":\"{}\",\"time\":{}}}",
            json_escape(name),
            json_escape(&e.path),
            e.opened
        ));
    }
    out.push(']');
    out
}

/// 轨道面板 JSON：`[{"index","name","port","channel","color":[r,g,b,a],"muted","soloed"}]`。
pub fn tracks_json() -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let mut out = String::from("[");
    for (i, t) in s.tracks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"index\":{},\"name\":\"{}\",\"port\":{},\"channel\":{},\"color\":[{},{},{},{}],\"muted\":{},\"soloed\":{}}}",
            i,
            json_escape(&t.name),
            t.port,
            t.channel,
            t.color[0],
            t.color[1],
            t.color[2],
            t.color[3],
            t.muted,
            t.soloed
        ));
    }
    out.push(']');
    out
}

/// 视图快照（Dart 文本浮层用），固定 12 个 f32：
/// `[mode, ppu, scroll_x, scroll_y, key_height, left_panel_w, lane_height,
///   total_ticks, ppq, key0_y, time_sig_num, time_sig_den]`
pub fn view_snapshot() -> [f32; 12] {
    let Ok(s) = scene().lock() else {
        return [0.0; 12];
    };
    let ppq = s.ppq as f32;
    match s.view_mode {
        ViewMode::Pianoroll => [
            1.0,
            s.pr_view.base.pixels_per_tick,
            s.pr_view.base.scroll_x,
            s.pr_view.base.scroll_y,
            s.pr_view.key_height,
            s.pr_view.keyboard_width(),
            0.0,
            s.total_ticks as f32,
            ppq,
            s.pr_view.key_to_y(0),
            4.0,
            4.0,
        ],
        ViewMode::Arrangement => [
            0.0,
            s.arr_view.base.pixels_per_tick,
            s.arr_view.base.scroll_x,
            s.arr_view.base.scroll_y,
            0.0,
            s.arr_view.base.left_panel_width,
            s.arr_view.lane_height(),
            s.total_ticks as f32,
            ppq,
            0.0,
            4.0,
            4.0,
        ],
    }
}

/// PR 自动化栏数据（仅 PB/CC/RPN 三类），JSON：
/// `[{"kind":0,"name":"Pitch Bend","points":[[tick,value],...]}, ...]`
pub fn automation_json(track: usize) -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let Some(t) = doc.data.model.tracks.get(track) else {
        return "[]".into();
    };
    let mut out = String::from("[");
    let mut first = true;
    for lane in &t.automation_lanes {
        let Some(kind) = lane_kind(&lane.target) else {
            continue;
        };
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            "{{\"kind\":{},\"name\":\"{}\",\"points\":[",
            kind,
            json_escape(&lane.target.display_name())
        ));
        for (i, e) in lane.events.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!("[{},{}]", e.tick, e.value));
        }
        out.push_str("]}");
    }
    out.push(']');
    out
}

/// 力度条数据（当前轨道，按 tick 聚合取最大力度），JSON：`[[tick, velocity01], ...]`
pub fn velocity_json(track: usize) -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let model = &doc.data.model;
    if model.tracks.get(track).is_none() {
        return "[]".into();
    }
    let mut points: Vec<(u32, u8)> = Vec::new();
    for key in 0..=yinhe_types::MAX_KEY {
        for note in model.key_notes(key).iter() {
            if note.track as usize == track {
                points.push((note.start_tick, note.velocity));
            }
        }
    }
    points.sort_unstable_by_key(|p| p.0);
    let mut out = String::from("[");
    let mut first = true;
    let mut i = 0;
    while i < points.len() {
        let tick = points[i].0;
        let mut vel = points[i].1;
        i += 1;
        while i < points.len() && points[i].0 == tick {
            vel = vel.max(points[i].1);
            i += 1;
        }
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!("[{tick},{}]", f32::from(vel) / 127.0));
    }
    out.push(']');
    out
}

/// 编辑自动化锚点（仅 PB/CC/RPN，离散）：把指定 lane 的 tick 值设为 value。
///
/// `lane_index` 为 1-based（与 `automation_json` 的输出顺序一致）。成功返回 1。
pub fn set_automation_value(track: usize, lane_index: usize, tick: u32, value: f32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let model = std::sync::Arc::make_mut(&mut doc.data.model);
    let Some(t) = model.tracks.get_mut(track) else {
        return 0;
    };
    let t = std::sync::Arc::make_mut(t);
    let mut seen = 0usize;
    let mut target = None;
    for lane in t.automation_lanes.iter_mut() {
        if lane_kind(&lane.target).is_some() {
            seen += 1;
            if seen == lane_index {
                target = Some(lane);
                break;
            }
        }
    }
    let Some(lane) = target else {
        return 0;
    };
    use yinhe_types::automation::{AutomationEvent, SegmentShape};
    lane.upsert(AutomationEvent {
        id: 0,
        tick,
        value: value.clamp(0.0, 1.0),
        shape: SegmentShape::Step,
    });
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    // 力度/CC 变化会影响发声：demo 阶段整模型重载。
    crate::audio::on_model_loaded(model);
    log::info!("自动化编辑: track={track} lane={lane_index} tick={tick} value={value:.3}");
    1
}

/// 自动化类型：0=PB、1=CC、2=RPN；其它（Nrpn/Tempo/插件参数）不展示。
fn lane_kind(target: &yinhe_types::automation::AutomationTarget) -> Option<u8> {
    use yinhe_types::automation::{AutomationTarget, ParamDevice, xsynth_param};
    match target {
        AutomationTarget::PitchBend => Some(0),
        AutomationTarget::CC { .. } => Some(1),
        AutomationTarget::Rpn { .. } => Some(2),
        AutomationTarget::Param {
            device: ParamDevice::ChannelInstrument { .. },
            id,
            ..
        } => match *id {
            xsynth_param::PITCH_BEND => Some(0),
            xsynth_param::PB_SENSITIVITY | xsynth_param::FINE_TUNE | xsynth_param::COARSE_TUNE => {
                Some(2)
            }
            _ => None,
        },
        _ => None,
    }
}

/// 极简 JSON 字符串转义（双引号/反斜杠/控制字符）。
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {}
            _ => out.push(c),
        }
    }
    out
}
