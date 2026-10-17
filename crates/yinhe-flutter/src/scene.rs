//! 渲染场景：文档快照（模型 + 视口 + 颜色）与演示内容生成。
//!
//! 渲染线程只读快照；触摸手势（`input.rs`）与首帧初始化写视口。
//! 不跨线程借用 `Document`，避免把 editor-core 的内部可变性带进渲染线程。

use std::sync::{Arc, Mutex, OnceLock};

use yinhe_core::{NoteEvent, YinModel};
use yinhe_editor_core::Document;
use yinhe_types::PianoRollView;

/// 渲染线程与触摸手势共享的场景数据。
pub struct RenderScene {
    /// 当前文档模型（None = 无文档，渲染清屏）。
    pub model: Option<Arc<YinModel>>,
    /// 视口（逻辑点坐标系）。
    pub view: PianoRollView,
    /// 轨道颜色（RGBA per track）。
    pub track_colors: Vec<[f32; 4]>,
    /// PR 可见轨（`EditState::pr_visible` 的语义：主轨强制可见）。
    pub track_visible: Vec<bool>,
    /// 总 tick 数（视口 clamp 用）。
    pub total_ticks: f64,
}

impl Default for RenderScene {
    fn default() -> Self {
        Self {
            model: None,
            view: PianoRollView::default(),
            track_colors: Vec::new(),
            track_visible: Vec::new(),
            total_ticks: 0.0,
        }
    }
}

static SCENE: OnceLock<Mutex<RenderScene>> = OnceLock::new();

/// 全局场景（渲染线程每帧取快照；触摸手势直接写视口）。
pub fn scene() -> &'static Mutex<RenderScene> {
    SCENE.get_or_init(|| Mutex::new(RenderScene::default()))
}

/// 生成演示文档并替换当前场景。
///
/// 阶段 B 的临时内容来源；SAF 文件打开接入后移除。
pub fn load_demo() {
    let mut doc = Document::empty();
    // 黑乐谱演示内容必然含重叠音符。
    doc.edit.allow_overlapping_notes = true;
    let gen_max = add_demo_notes(&mut doc);

    // 轨道颜色/显隐（conductor 色用中性浅灰，与主题无关的演示值）。
    doc.sync_track_caches_with_conductor_color([0.85, 0.85, 0.88, 1.0]);

    let model_arc = doc.data.model.clone();
    let total_ticks = (model_arc.tick_length as f64)
        .max(f64::from(gen_max))
        .max(1.0);

    let mut view = PianoRollView::default();
    // 初始缩放：全曲约 2 屏宽（沿用旧安卓壳 set_model 的算法）。
    view.base.pixels_per_tick = (1280.0 * 2.0 / total_ticks as f32).clamp(0.001, 10.0);
    // viewport_h = 0：首帧渲染时由 clamp_scroll 完成键盘居中初始化。

    let colors = doc.edit.track_cache.colors.clone();
    let visible = doc.edit.pr_visible();

    let Ok(mut s) = scene().lock() else { return };
    s.model = Some(model_arc);
    s.view = view;
    s.track_colors = colors;
    s.track_visible = visible;
    s.total_ticks = total_ticks;

    log::info!(
        "演示文档就绪: {} 音符, {} tick",
        s.model.as_ref().map_or(0, |m| m.note_count),
        total_ticks as u64
    );
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
