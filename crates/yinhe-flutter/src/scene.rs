//! 渲染场景：文档快照（模型 + PR/AR 双视口 + 颜色）与演示内容生成。
//!
//! 渲染线程只读快照；触摸手势（`input.rs`）与首帧初始化写视口。
//! 不跨线程借用 `Document`，避免把 editor-core 的内部可变性带进渲染线程。

use std::sync::{Arc, Mutex, OnceLock};

use yinhe_core::{NoteEvent, YinModel};
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

    apply_document(&doc, f64::from(gen_max));
}

/// 后台加载用户选择的 MIDI 文件并替换当前场景。
pub fn open_midi(path: String) {
    let spawn = std::thread::Builder::new()
        .name("yinhe-midi-load".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || match load_document(&path) {
            Ok(doc) => apply_document(&doc, 0.0),
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
    Ok(doc)
}

/// 把文档写入场景快照（演示与文件加载共用）。
///
/// `min_ticks`：演示内容生成时模型 tick_length 尚未刷新，用它兜底总长。
fn apply_document(doc: &Document, min_ticks: f64) {
    let model = doc.data.model.clone();
    let total_ticks = (model.tick_length as f64).max(min_ticks).max(1.0);

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

    let colors = doc.edit.track_cache.colors.clone();
    let visible = doc.edit.pr_visible();
    let num_tracks = model.tracks.len();
    let note_count = model.note_count;

    let Ok(mut s) = scene().lock() else {
        return;
    };
    s.model = Some(model.clone());
    s.pr_view = pr_view;
    s.arr_view = arr_view;
    s.view_mode = ViewMode::Arrangement;
    s.track_colors = colors;
    s.track_visible = visible;
    s.total_ticks = total_ticks;
    s.num_tracks = num_tracks;
    drop(s);

    log::info!("文档就绪: {note_count} 音符, {total_ticks:.0} tick, {num_tracks} 轨");
    crate::audio::on_model_loaded(model);
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
