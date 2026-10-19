//! 渲染场景：文档快照（模型 + PR/AR 双视口 + 颜色）与演示内容生成。
//!
//! 渲染线程只读快照；触摸手势（`input.rs`）与首帧初始化写视口。
//! 不跨线程借用 `Document`，避免把 editor-core 的内部可变性带进渲染线程。

use std::sync::{Arc, Mutex, OnceLock};

use yinhe_core::YinModel;
use yinhe_editor_core::Document;
use yinhe_editor_core::clipboard::{NotesClipboard, PasteMode};
use yinhe_editor_core::follow::total_ticks_padded;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::NoteSource;
use yinhe_types::{ArrangementView, PianoRollView, TimelineViewBase};

/// 视图模式（与 Dart 约定一致：0=AR 走带，1=PR 卷帘）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Arrangement = 0,
    Pianoroll = 1,
}

/// 小节线感知吸附上下文：网格相对当前小节起点对齐，拍号变化段不会错位
/// （与桌面 `yinhe_editor_core::quantize::snap_tick*` 同规则）。
#[derive(Clone)]
pub struct SnapCtx {
    ppq: u32,
    num: u8,
    den_pow: u8,
    events: Vec<yinhe_types::TimeSigEvent>,
}

impl SnapCtx {
    pub fn new(s: &RenderScene) -> Self {
        match s.model.as_ref() {
            Some(m) => Self {
                ppq: s.ppq.max(1),
                num: m.tempo_map.time_sig_default.0,
                den_pow: m.tempo_map.time_sig_default.1,
                events: m.tempo_map.time_sig_events.clone(),
            },
            None => Self {
                ppq: s.ppq.max(1),
                num: 4,
                den_pow: 2,
                events: Vec::new(),
            },
        }
    }

    fn args(&self) -> (u32, u8, u8, &[yinhe_types::TimeSigEvent]) {
        (self.ppq, self.num, self.den_pow, &self.events)
    }

    /// 就近吸附（round）。
    pub fn round(&self, q: QuantizePreset, tick: f64) -> f64 {
        let (tpb, num, den, ev) = self.args();
        yinhe_editor_core::quantize::snap_tick(tick, q, tpb, Some((tpb, num, den, ev))).max(0.0)
    }

    /// 向上吸附（ceil）。
    pub fn ceil(&self, q: QuantizePreset, tick: f64) -> f64 {
        let (tpb, num, den, ev) = self.args();
        yinhe_editor_core::quantize::snap_tick_ceil(tick, q, tpb, Some((tpb, num, den, ev)))
            .max(0.0)
    }

    /// 向下吸附（floor）。
    pub fn floor(&self, q: QuantizePreset, tick: f64) -> f64 {
        let (tpb, num, den, ev) = self.args();
        yinhe_editor_core::quantize::snap_tick_floor(tick, q, tpb, Some((tpb, num, den, ev)))
            .max(0.0)
    }
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
    /// 所属通道的默认乐器（-1 = 未设置）。
    pub instrument_bank: i32,
    pub instrument_program: i32,
    /// 是否 Master（Conductor）轨。
    pub master: bool,
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
    /// “显示轨道”过滤：Some = 只画该轨（曲名框选择）。
    pub display_track: Option<usize>,
    /// 当前工具（PR：5 种；AR 只用选框/手形）。
    pub tool: Tool,
    /// 当前编辑轨（AR 进入 PR 的轨；工具写入目标）。
    pub active_track: Option<usize>,
    /// 非播放态的播放光标位置（点击跳转用，tick）。
    pub cursor_tick: f64,
    /// 覆盖层（选框/预览）修订号与内容（画布内容逻辑坐标）。
    pub overlay_rev: u32,
    pub overlay_box: Option<[f32; 4]>,
    pub overlay_cells: Vec<[f32; 4]>,
    /// 文档本体（编辑操作：轨道排序等）。
    pub document: Option<Document>,
    /// 文档/选区世代号：每次 refresh_snapshot 自增，Dart 据此刷新轨道/自动化浮层。
    pub doc_generation: u32,
}

/// 工具（移动端子集：与桌面 `tools_panel::Tool` 的编号对齐）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select = 0,
    Pan = 1,
    Pencil = 2,
    Eraser = 3,
    Brush = 4,
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
            display_track: None,
            tool: Tool::Select,
            active_track: None,
            cursor_tick: 0.0,
            overlay_rev: 0,
            overlay_box: None,
            overlay_cells: Vec::new(),
            document: None,
            doc_generation: 0,
        }
    }
}

static SCENE: OnceLock<Mutex<RenderScene>> = OnceLock::new();

/// 全局场景（渲染线程每帧取快照；触摸手势直接写视口）。
pub fn scene() -> &'static Mutex<RenderScene> {
    SCENE.get_or_init(|| Mutex::new(RenderScene::default()))
}

/// 后台加载用户选择的文件（.mid / .ying）并替换当前场景。
pub fn open_midi(path: String) {
    let spawn = std::thread::Builder::new()
        .name("yinhe-midi-load".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || match load_document(&path) {
            Ok(doc) => apply_document(doc, 0.0),
            Err(e) => log::error!("文件加载失败（{path}）: {e}"),
        });
    if let Err(e) = spawn {
        log::error!("加载线程创建失败: {e}");
    }
}

/// 按扩展名解析文件并组装为可渲染/可播放的文档（.ying 工程 / MIDI）。
fn load_document(path: &str) -> Result<Document, String> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".ying") {
        return load_ying_document(path);
    }
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

/// 解析 `.ying` 工程文件（ying = 移动端工程归档，容器与 .yin 兼容）。
fn load_ying_document(path: &str) -> Result<Document, String> {
    let started = std::time::Instant::now();
    let (model, _sf, _mapping, _mixer) =
        yinhe_yin::load_yin_with_sf(path).map_err(|e| format!("工程解析失败: {e}"))?;
    log::info!(
        "工程解析完成: {} 音符，用时 {:?}",
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
    // 打开既有工程视为已保存（dirty 起点干净）。
    doc.mark_saved();
    doc.file_path = Some(path.to_string());
    doc.sync_track_caches_with_conductor_color([0.85, 0.85, 0.88, 1.0]);
    push_recent(path);
    Ok(doc)
}

/// 新建空工程（17 轨、120BPM、4/4；等同桌面 `Document::empty()`）。
pub fn new_project() -> i32 {
    let doc = Document::empty();
    // 空工程给 4 小节的初始时间轴长度，避免除零缩放。
    apply_document(doc, 1920.0);
    1
}

/// 保存为 `.ying` 工程归档（写到给定路径，签名与桌面 .yin 容器一致）。
pub fn save_ying(path: String) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    match yinhe_yin::save_yin(&doc.data.model, &path) {
        Ok(()) => {
            // 记录落盘路径：`is_dirty()` 用它判断"尚未落盘"。
            doc.file_path = Some(path.clone());
            log::info!("工程已导出: {path}");
            1
        }
        Err(e) => {
            log::error!("工程导出失败: {e}");
            0
        }
    }
}

/// 保存完成后标记工程状态（清除 dirty）。
pub fn mark_saved() -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    doc.mark_saved();
    1
}

/// 工程状态：`[can_undo, can_redo, dirty]`。
pub fn doc_state() -> [i32; 3] {
    let Ok(s) = scene().lock() else {
        return [0, 0, 0];
    };
    let Some(doc) = s.document.as_ref() else {
        return [0, 0, 0];
    };
    [
        i32::from(doc.undo_past_len() > 0),
        i32::from(!doc.redo_labels().is_empty()),
        i32::from(doc.is_dirty()),
    ]
}

/// 设置当前工具（0=选框 1=手形 2=铅笔 3=橡皮 4=刷子）。
pub fn set_tool(tool: i32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    s.tool = match tool {
        1 => Tool::Pan,
        2 => Tool::Pencil,
        3 => Tool::Eraser,
        4 => Tool::Brush,
        _ => Tool::Select,
    };
    1
}

/// 设置量化吸附档位：kind 0=Off 1=Auto 2=Fraction(num/den) 3=Absolute(ticks)。
pub fn set_quantize(kind: i32, num: u32, den: u32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let preset = match kind {
        0 => QuantizePreset::Off,
        2 => QuantizePreset::Fraction(num.max(1), den.max(1)),
        3 => QuantizePreset::Absolute(num.max(1)),
        _ => QuantizePreset::Auto,
    };
    doc.edit.quantize_pianoroll = preset;
    doc.edit.quantize_arrange = preset;
    1
}

/// 设置当前编辑轨（-1 = 无）。
pub fn set_active_track(track: i32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    s.active_track = if track < 0 {
        None
    } else {
        Some(track as usize)
    };
    log::info!("当前编辑轨: {track}");
    1
}

/// 文档/选区世代号（Dart 在变化时刷新轨道/自动化元数据）。
pub fn document_generation() -> u32 {
    scene().lock().map(|s| s.doc_generation).unwrap_or(0)
}

/// 覆盖层修订号（Dart 仅在变化时重取 JSON）。
pub fn overlay_revision() -> u32 {
    scene().lock().map(|s| s.overlay_rev).unwrap_or(0)
}

/// 覆盖层 JSON：`{"box":[x0,y0,x1,y1]|null,"cells":[...],"sel":[...]}`。
///
/// 坐标均为画布内容逻辑坐标（Flutter 侧直接绘制）。
pub fn overlay_json() -> String {
    let Ok(s) = scene().lock() else {
        return "{}".into();
    };
    let mut out = String::from("{\"box\":");
    match s.overlay_box {
        Some(b) => out.push_str(&format!("[{},{},{},{}]", b[0], b[1], b[2], b[3])),
        None => out.push_str("null"),
    }
    out.push_str(",\"cells\":[");
    for (i, c) in s.overlay_cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("[{},{},{},{}]", c[0], c[1], c[2], c[3]));
    }
    out.push_str("],\"sel\":[");
    let mut first = true;
    if let Some(doc) = s.document.as_ref() {
        match s.view_mode {
            ViewMode::Pianoroll => {
                let view = &s.pr_view;
                for (t0, t1, klo, khi) in doc.edit.sel_rect.effective_rects() {
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    let x0 = view.tick_to_x(t0);
                    let x1 = view.tick_to_x(t1);
                    let y0 = view.key_to_y(khi);
                    let y1 = view.key_to_y(klo) + view.key_height;
                    out.push_str(&format!("[{x0},{y0},{x1},{y1}]"));
                }
            }
            ViewMode::Arrangement => {
                let view = &s.arr_view;
                let lane_h = view.lane_height();
                for (t0, t1, tr_lo, tr_hi) in &doc.edit.arr_sel_rect {
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    let x0 = view.base.tick_to_x(*t0);
                    let x1 = view.base.tick_to_x(*t1);
                    let y0 = *tr_lo as f32 * lane_h - view.base.scroll_y;
                    let y1 = (*tr_hi as f32 + 1.0) * lane_h - view.base.scroll_y;
                    out.push_str(&format!("[{x0},{y0},{x1},{y1}]"));
                }
            }
        }
    }
    out.push_str("]}");
    out
}

/// 工程元数据 JSON（工程设置对话框回显）。
pub fn project_meta_json() -> String {
    let Ok(s) = scene().lock() else {
        return "{}".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "{}".into();
    };
    let m = &doc.data.model.meta;
    format!(
        "{{\"name\":\"{}\",\"artist\":\"{}\",\"description\":\"{}\",\"ppq\":{}}}",
        json_escape(&m.name),
        json_escape(&m.artist),
        json_escape(&m.description),
        m.ppq
    )
}

/// 工程元数据编辑（歌名/艺术家/简介/PPQ），差异字段合并为一条 undo。
pub fn set_project_meta(name: String, artist: String, description: String, ppq: u32) -> i32 {
    use yinhe_editor_core::history::UndoAction;

    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    let mut actions: Vec<UndoAction> = Vec::new();
    let mut ppq_change: Option<(u32, u32)> = None;
    {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        let has_notes = model.note_count > 0;
        if model.meta.name != name {
            actions.push(UndoAction::ProjectName {
                old: model.meta.name.clone(),
                new: name.clone(),
            });
            model.meta.name = name.clone();
            // 与桌面 apply 同规则：歌曲名同步到 track 0 名称。
            if let Some(t) = model.tracks.get_mut(0) {
                std::sync::Arc::make_mut(t).name = name.clone();
            }
        }
        if model.meta.artist != artist {
            actions.push(UndoAction::ProjectArtist {
                old: model.meta.artist.clone(),
                new: artist.clone(),
            });
            model.meta.artist = artist;
        }
        if model.meta.description != description {
            actions.push(UndoAction::ProjectDescription {
                old: model.meta.description.clone(),
                new: description.clone(),
            });
            model.meta.description = description;
        }
        let old_ppq = model.meta.ppq;
        if old_ppq != ppq && ppq > 0 {
            if has_notes {
                // 有音符：缩放全部 tick 字段（与桌面一致）。
                model.rescale_ppq(ppq);
            } else {
                model.meta.ppq = ppq;
                model.commit_automation(&yinhe_types::automation::AutomationTarget::Tempo);
            }
            ppq_change = Some((old_ppq, ppq));
        }
    }
    if let Some((old, new)) = ppq_change {
        let has_notes = doc.data.model.note_count > 0;
        actions.push(UndoAction::ProjectPpq {
            old,
            new,
            rescale: has_notes,
        });
    }
    if actions.is_empty() {
        return 0;
    }
    let action = if actions.len() == 1 {
        actions.pop().unwrap_or(UndoAction::ProjectName {
            old: String::new(),
            new: String::new(),
        })
    } else {
        UndoAction::Composite(actions)
    };
    doc.push_undo(action, "Edit project meta", before);
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::on_model_loaded(model);
    log::info!("工程元数据已更新");
    1
}

/// 把文档写入场景快照（演示与文件加载共用）。
///
/// `min_ticks`：演示内容生成时模型 tick_length 尚未刷新，用它兜底总长。
fn apply_document(doc: Document, min_ticks: f64) {
    // 初始缩放只看实际内容（不含尾部 64 小节，避免打开时音符变小）；
    // 可视总长额外预留 64 小节（与桌面 follow::total_ticks_padded 一致）。
    let content_ticks = (doc.data.model.tick_length as f64).max(min_ticks).max(1.0);
    let total_ticks = total_ticks_padded(doc.data.model.tick_length, doc.data.model.meta.ppq)
        .max(min_ticks)
        .max(1.0);

    let mut pr_view = PianoRollView::default();
    // 初始缩放：全曲约 2 屏宽，再放大 1.3 倍（进入钢琴卷帘更舒适）。
    pr_view.base.pixels_per_tick = (1280.0 * 2.0 / content_ticks as f32 * 1.3).clamp(0.001, 10.0);

    let arr_base = TimelineViewBase {
        pixels_per_tick: (1280.0 * 2.0 / content_ticks as f32).clamp(0.0005, 8.0),
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
pub(crate) fn refresh_snapshot(s: &mut RenderScene) {
    s.doc_generation = s.doc_generation.wrapping_add(1);
    // 覆盖层（选框/预览）随快照变化一起失效（undo/编辑/加载后重取）。
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
    // 文档修订号：编辑后（视图未变）也必须让音符层缓存失效。
    if let Some(doc) = s.document.as_mut() {
        doc.data.bump_revision();
    }
    let Some(doc) = s.document.as_ref() else {
        return;
    };
    let model = doc.data.model.clone();
    let colors = doc.edit.track_cache.colors.clone();
    // 内容结束后实时保留 64 小节空余（编辑后立即可滚动/点击）。
    s.total_ticks = total_ticks_padded(model.tick_length, model.meta.ppq).max(1.0);
    s.model = Some(model.clone());
    s.track_colors = colors.clone();
    s.track_visible = doc.edit.pr_visible();
    s.num_tracks = model.tracks.len();
    s.ppq = model.meta.ppq;
    s.tracks = model
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let inst = model
                .channel_instruments
                .get(t.channel as usize)
                .copied()
                .flatten();
            TrackInfoLite {
                name: t.name.to_string(),
                port: t.port,
                channel: t.channel,
                instrument_bank: inst.map_or(-1, |v| i32::from(v.bank_msb)),
                instrument_program: inst.map_or(-1, |v| i32::from(v.program)),
                master: t.kind == yinhe_core::TrackKind::Master || i == 0,
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
            }
        })
        .collect();
}

/// AR 长按拖动排序：把 `from` 轨移动到 `to` 位置（一步一 undo）。成功返回 1。
pub fn move_track(from: usize, to: usize) -> i32 {
    // Conductor（Master）固定在 0 号位，不参与排序。
    if from == 0 || to == 0 {
        return 0;
    }
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    {
        let Some(doc) = s.document.as_mut() else {
            return 0;
        };
        let before = doc.capture_snapshot();
        let Some(action) = doc.move_track(from, to) else {
            return 0;
        };
        doc.push_undo(action, "undo.move_track", before);
    }
    // 面板索引随轨道顺序变化：活动/过滤轨同步 remap。
    remap_scene_track_index(&mut s, from, to);
    let mask = skip_mask(&s);
    refresh_snapshot(&mut s);
    let model = s.model.clone();
    drop(s);
    if let Some(model) = model {
        crate::audio::reload_notes(model);
    }
    crate::audio::set_skip_mask(mask);
    log::info!("轨道已移动: {from} -> {to}");
    1
}

/// 删除音轨（连同其音符；一步一 undo）。成功返回 1。
pub fn delete_track(idx: usize) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    {
        let Some(doc) = s.document.as_mut() else {
            return 0;
        };
        let before = doc.capture_snapshot();
        let Some(action) = doc.remove_track(idx) else {
            return 0;
        };
        doc.push_undo(action, "undo.delete_track", before);
    }
    let num = s
        .document
        .as_ref()
        .map(|d| d.data.model.tracks.len())
        .unwrap_or(0);
    // 活动/过滤轨越界或指向被删轨时收敛。
    let remap = |slot: &mut Option<usize>| {
        if let Some(t) = *slot {
            if t == idx {
                *slot = None;
            } else if t > idx {
                *slot = Some(t - 1);
            } else if t >= num {
                *slot = None;
            }
        }
    };
    remap(&mut s.active_track);
    remap(&mut s.display_track);
    let mask = skip_mask(&s);
    refresh_snapshot(&mut s);
    let model = s.model.clone();
    drop(s);
    if let Some(model) = model {
        crate::audio::reload_notes(model);
    }
    crate::audio::set_skip_mask(mask);
    log::info!("音轨已删除: {idx}");
    1
}

/// 轨道移动后把场景里的活动轨/过滤轨索引按同一规则 remap。
fn remap_scene_track_index(s: &mut RenderScene, from: usize, to: usize) {
    let map = |t: usize| -> usize {
        if t == from {
            to
        } else if from < to && t > from && t <= to {
            t - 1
        } else if from > to && t >= to && t < from {
            t + 1
        } else {
            t
        }
    };
    s.active_track = s.active_track.map(map);
    s.display_track = s.display_track.map(map);
}

/// 播放光标在当前视图下的 x（逻辑像素；-1 = 无文档）。
///
/// 坐标原点与画布一致（含键盘列/轨道面板偏移），Dart 直接画竖线。
pub fn playhead_x() -> f32 {
    // 注意锁顺序：playback_tick 内部会锁 scene，必须先算 tick 再锁 scene
    // （持 scene 锁调用 audio 会自锁死）。
    let playing = crate::audio::is_playing();
    let tick = if playing {
        crate::audio::playback_tick()
    } else {
        let Ok(s) = scene().lock() else {
            return -1.0;
        };
        s.cursor_tick
    };
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

/// 设置播放光标（点击跳转）：吸附到量化网格并 seek 音频。
pub fn set_play_cursor(tick: f64) {
    let snapped = {
        let Ok(s) = scene().lock() else {
            return;
        };
        let Some(doc) = s.document.as_ref() else {
            return;
        };
        let q = match s.view_mode {
            ViewMode::Pianoroll => doc.edit.quantize_pianoroll,
            ViewMode::Arrangement => doc.edit.quantize_arrange,
        };
        let ppu = match s.view_mode {
            ViewMode::Pianoroll => s.pr_view.base.pixels_per_tick,
            ViewMode::Arrangement => s.arr_view.base.pixels_per_tick,
        };
        let q = q.resolved(ppu, s.ppq);
        SnapCtx::new(&s)
            .round(q, tick.max(0.0))
            .clamp(0.0, (s.total_ticks - 1.0).max(0.0))
    };
    if let Ok(mut s) = scene().lock() {
        s.cursor_tick = snapped;
    }
    crate::audio::seek(snapped);
}

/// 停止播放时把光标归零（与走带行为一致）。
pub fn reset_cursor() {
    if let Ok(mut s) = scene().lock() {
        s.cursor_tick = 0.0;
    }
}

// ── 最近文件（主页列表，持久化到私有目录 recent.txt）──

struct RecentEntry {
    path: String,
    opened: u64,
}

static RECENT: OnceLock<Mutex<Vec<RecentEntry>>> = OnceLock::new();
static FILES_DIR: OnceLock<Mutex<Option<String>>> = OnceLock::new();

/// 应用内音符剪贴板（复制快照 + 选区，O(1) 结构共享）。
static CLIPBOARD: Mutex<Option<NotesClipboard>> = Mutex::new(None);

fn recent_slot() -> &'static Mutex<Vec<RecentEntry>> {
    RECENT.get_or_init(|| Mutex::new(Vec::new()))
}

fn files_dir_slot() -> &'static Mutex<Option<String>> {
    FILES_DIR.get_or_init(|| Mutex::new(None))
}

/// Kotlin 侧传入私有目录（用于最近文件/设置持久化）。
pub fn set_files_dir(dir: String) {
    if let Ok(mut g) = files_dir_slot().lock() {
        *g = Some(dir);
    }
    load_recent();
    load_settings();
}

/// 设置文件写入（音色库 + 音频参数持久化）。
pub fn save_soundfont_settings() {
    write_settings();
}

/// 音频设置变更后持久化（设置页调用）。
pub fn save_audio_settings() {
    write_settings();
}

fn write_settings() {
    let Some(dir) = files_dir_slot().lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    let mut text = String::new();
    // 音色库表：每行一个 `soundfont=<0|1>|<path>`（0/1 = 停用/启用）。
    for e in crate::audio::soundfonts() {
        text.push_str(&format!(
            "soundfont={}|{}\n",
            if e.enabled { 1 } else { 0 },
            e.path
        ));
    }
    let a = crate::audio::settings();
    text.push_str(&format!(
        "max_voices={}\nbuffer_size={}\nignore_velocity={}\n",
        a.max_voices, a.buffer_size, a.ignore_velocity
    ));
    if let Err(e) = std::fs::write(format!("{dir}/settings.txt"), text) {
        log::error!("设置写入失败: {e}");
    }
}

/// 读取并应用持久化设置（启动时由 `set_files_dir` 触发）。
fn load_settings() {
    let Some(dir) = files_dir_slot().lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(format!("{dir}/settings.txt")) else {
        return;
    };
    let mut max_voices = crate::audio::AudioSettings::default().max_voices;
    let mut buffer_size = crate::audio::AudioSettings::default().buffer_size;
    let mut ignore_velocity = crate::audio::AudioSettings::default().ignore_velocity;
    let mut soundfonts: Vec<crate::audio::SoundfontEntry> = Vec::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("soundfont=") {
            // 新格式 `1|path`；旧格式 `path`（视为启用）。
            let (enabled, path) = match v.split_once('|') {
                Some((flag, path)) => (flag != "0", path),
                None => (true, v),
            };
            if !path.is_empty() {
                soundfonts.push(crate::audio::SoundfontEntry {
                    path: path.to_string(),
                    enabled,
                });
            }
        } else if let Some(v) = line.strip_prefix("max_voices=") {
            max_voices = v.trim().parse().unwrap_or(max_voices);
        } else if let Some(v) = line.strip_prefix("buffer_size=") {
            buffer_size = v.trim().parse().unwrap_or(buffer_size);
        } else if let Some(v) = line.strip_prefix("ignore_velocity=") {
            ignore_velocity = v.trim().parse().unwrap_or(ignore_velocity);
        }
    }
    if !soundfonts.is_empty() {
        log::info!("恢复音色库设置: {} 个", soundfonts.len());
        crate::audio::set_soundfonts(soundfonts);
    }
    crate::audio::set_settings(max_voices, buffer_size, ignore_velocity);
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
            "{{\"index\":{},\"name\":\"{}\",\"port\":{},\"channel\":{},\"color\":[{},{},{},{}],\"muted\":{},\"soloed\":{},\"ib\":{},\"ip\":{},\"master\":{}}}",
            i,
            json_escape(&t.name),
            t.port,
            t.channel,
            t.color[0],
            t.color[1],
            t.color[2],
            t.color[3],
            t.muted,
            t.soloed,
            t.instrument_bank,
            t.instrument_program,
            t.master
        ));
    }
    out.push(']');
    out
}

/// 视图快照（Dart 文本浮层用），固定 12 个 f32：
/// `[mode, ppu, scroll_x, scroll_y, key_height, left_panel_w, lane_height,
///   total_ticks, ppq, key0_y, time_sig_num, time_sig_den]`
pub fn view_snapshot() -> [f32; 13] {
    let Ok(s) = scene().lock() else {
        return [0.0; 13];
    };
    let ppq = s.ppq as f32;
    // 顶栏 BPM/拍号 = 播放光标处的实时值（原生事件优先，否则工程默认），
    // 与桌面时间码 `bpm_at_time` / `time_sig_at_tick` 同规则。
    let (sig_num, sig_den, bpm) = match s.document.as_ref() {
        Some(doc) => {
            let model = &doc.data.model;
            let tick = crate::audio::playback_tick_for(model).max(0.0);
            let (num, den) = model.tempo_map.time_sig_at_tick(tick as u32);
            (num, den, model.tempo_map.bpm_at_tick(tick as u64))
        }
        None => (4, 2, 120.0),
    };
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
            f32::from(sig_num),
            f32::from(sig_den),
            bpm,
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
            f32::from(sig_num),
            f32::from(sig_den),
            bpm,
        ],
    }
}

/// 拍号分段 JSON：`[[tick,num,den_pow], ...]`（首段从 0 开始）。
///
/// 标尺/网格线按分段画（拍号变化时各段用各自的 measure 长）。
pub fn time_sig_segments_json() -> String {
    use yinhe_types::time_format::build_time_sig_segments;
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let tm = &doc.data.model.tempo_map;
    let (def_num, def_den) = tm.time_sig_default;
    let segments = build_time_sig_segments(&tm.time_sig_events, def_num, def_den);
    let mut out = String::with_capacity(segments.len() * 16 + 2);
    out.push('[');
    for (i, (tick, num, den_pow)) in segments.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("[{tick},{num},{den_pow}]"));
    }
    out.push(']');
    out
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

/// 力度条数据（所有可见轨），JSON：`[[tick, length, velocity01, track], ...]`。
///
/// 与桌面一致：velocity <= 1 过滤；ppu 较小时按 `SUMMARY_BLOCK_TICKS`
/// 对每个 (块, 轨道) 取 gate 最长的代表音符（LOD 摘要）。
pub fn velocity_json() -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let model = &doc.data.model;
    let visible = doc.edit.pr_visible();
    let ppu = s.pr_view.base.pixels_per_tick;
    let block = yinhe_wgpu::pianoroll::select_summary_level(ppu)
        .map(|l| yinhe_wgpu::pianoroll::SUMMARY_BLOCK_TICKS[l]);

    let mut bars: Vec<(u32, u32, u8, u16)> = Vec::new();
    match block {
        None => {
            for key in 0..=yinhe_types::MAX_KEY {
                for note in model.key_notes(key).iter() {
                    if note.velocity <= 1 {
                        continue;
                    }
                    let t = note.track as usize;
                    if !visible.get(t).copied().unwrap_or(true) {
                        continue;
                    }
                    bars.push((
                        note.start_tick,
                        note.end_tick.saturating_sub(note.start_tick),
                        note.velocity,
                        note.track,
                    ));
                }
            }
        }
        Some(block) => {
            use std::collections::HashMap;
            // (块序号, 轨道) → (tick, gate, velocity) 代表音符。
            let mut map: HashMap<(u32, u16), (u32, u32, u8)> = HashMap::new();
            for key in 0..=yinhe_types::MAX_KEY {
                for note in model.key_notes(key).iter() {
                    if note.velocity <= 1 {
                        continue;
                    }
                    let t = note.track as usize;
                    if !visible.get(t).copied().unwrap_or(true) {
                        continue;
                    }
                    let len = note.end_tick.saturating_sub(note.start_tick);
                    let entry = map.entry((note.start_tick / block, note.track)).or_insert((
                        note.start_tick,
                        len,
                        note.velocity,
                    ));
                    if len > entry.1 || (len == entry.1 && note.velocity > entry.2) {
                        *entry = (note.start_tick, len, note.velocity);
                    }
                }
            }
            bars.extend(
                map.into_iter()
                    .map(|((_, track), (tick, len, vel))| (tick, len, vel, track)),
            );
        }
    }
    bars.sort_unstable_by_key(|b| (b.0, b.3));
    let mut out = String::with_capacity(bars.len() * 24 + 2);
    out.push('[');
    for (i, (tick, len, vel, track)) in bars.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let value01 = (f32::from(*vel) - 1.0) / 126.0;
        out.push_str(&format!("[{tick},{len},{value01:.4},{track}]"));
    }
    out.push(']');
    out
}

/// 复制当前选中音符到应用内剪贴板（快照 O(1)）。成功 1。
pub fn copy_selection() -> i32 {
    let Ok(s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_ref() else {
        return 0;
    };
    if doc.edit.selected.is_empty() {
        return 0;
    }
    let clipboard =
        NotesClipboard::from_snapshot(doc.data.model.clone(), doc.edit.selected.clone());
    drop(s);
    let Ok(mut slot) = CLIPBOARD.lock() else {
        return 0;
    };
    *slot = Some(clipboard);
    1
}

/// 剪贴板是否为空（Dart 菜单置灰用）。
pub fn clipboard_empty() -> bool {
    let Ok(slot) = CLIPBOARD.lock() else {
        return true;
    };
    match slot.as_ref() {
        Some(clipboard) => clipboard.is_empty(),
        None => true,
    }
}

/// 画布逻辑 x → 当前视图 tick（按当前量化档吸附，与点击跳转同规则）。
pub fn tick_at_x(s: &RenderScene, x: f32) -> f64 {
    let (ppu, scroll_x, origin) = match s.view_mode {
        ViewMode::Pianoroll => (
            s.pr_view.base.pixels_per_tick,
            s.pr_view.base.scroll_x,
            s.pr_view.keyboard_width(),
        ),
        ViewMode::Arrangement => (
            s.arr_view.base.pixels_per_tick,
            s.arr_view.base.scroll_x,
            s.arr_view.base.left_panel_width,
        ),
    };
    let raw = f64::from((x - origin + scroll_x) / ppu.max(0.0001)).max(0.0);
    let Some(doc) = s.document.as_ref() else {
        return raw;
    };
    let q = match s.view_mode {
        ViewMode::Pianoroll => doc.edit.quantize_pianoroll,
        ViewMode::Arrangement => doc.edit.quantize_arrange,
    };
    let q = q.resolved(ppu, s.ppq);
    SnapCtx::new(s)
        .round(q, raw)
        .clamp(0.0, (s.total_ticks - 1.0).max(0.0))
}

/// 在长按位置粘贴剪贴板（按当前量化吸附）。成功 1。
pub fn paste_selection(x: f32) -> i32 {
    let Some(clipboard) = CLIPBOARD.lock().ok().and_then(|slot| slot.clone()) else {
        return 0;
    };
    if clipboard.is_empty() {
        return 0;
    }
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    // 先按当前视口算出长按位置对应的 tick，并用当前量化档吸附。
    let tick = tick_at_x(&s, x);

    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let track_selected = doc.edit.track_selected.clone();
    let before = doc.capture_snapshot();
    let Some((action, _span)) =
        doc.paste_notes(&clipboard, tick, &track_selected, PasteMode::AtCursor)
    else {
        return 0;
    };
    doc.push_undo(action, "undo.paste", before);

    // paste_notes 只更新成员集；这里同步 PR/AR 选框，粘贴后可直接再次长按复制。
    let rects = doc.edit.selected.rects.clone();
    doc.edit.sel_rect.clear();
    doc.edit.arr_sel_rect.clear();
    for (t0, t1, klo, khi, trlo, trhi) in rects {
        doc.edit
            .sel_rect
            .push_rect((f64::from(t0), f64::from(t1), klo, khi), false);
        doc.edit
            .arr_sel_rect
            .push((f64::from(t0), f64::from(t1), trlo as usize, trhi as usize));
    }
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    set_play_cursor(tick);
    log::info!("已粘贴剪贴板到 tick {tick:.0}");
    1
}

/// 通道级默认乐器 JSON：16 项，`[bank, program]` 或 null。
pub fn channel_instruments_json() -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let mut out = String::from("[");
    for (i, inst) in doc.data.model.channel_instruments.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match inst {
            Some(v) => out.push_str(&format!("[{},{}]", v.bank_msb, v.program)),
            None => out.push_str("null"),
        }
    }
    out.push(']');
    out
}

/// 轨道音频 skip 掩码：mute + solo（有任意 solo 时只放行 solo 轨）+ 空音频轨。
fn skip_mask(s: &RenderScene) -> Vec<bool> {
    let Some(doc) = s.document.as_ref() else {
        return Vec::new();
    };
    let any_solo = doc
        .edit
        .track_overrides
        .iter()
        .enumerate()
        .any(|(i, o)| o.soloed && i < doc.data.model.tracks.len());
    doc.data
        .model
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let o = doc.edit.track_overrides.get(i);
            let muted = o.map(|o| o.muted).unwrap_or(false);
            let soloed = o.map(|o| o.soloed).unwrap_or(false);
            (t.kind == yinhe_core::TrackKind::Audio && t.audio_clips.is_empty())
                || muted
                || (any_solo && !soloed)
        })
        .collect()
}

fn toggle_override(track: usize, mute: bool) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if track >= doc.data.model.tracks.len() {
        return 0;
    }
    while doc.edit.track_overrides.len() <= track {
        doc.edit.track_overrides.push(Default::default());
    }
    {
        let o = &mut doc.edit.track_overrides[track];
        if mute {
            o.muted = !o.muted;
        } else {
            o.soloed = !o.soloed;
        }
    }
    let mask = skip_mask(&s);
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::set_skip_mask(mask);
    1
}

/// 切换轨道 mute（不参与 undo，与桌面一致）。成功 1。
pub fn toggle_track_mute(track: usize) -> i32 {
    toggle_override(track, true)
}

/// 切换轨道 solo（不参与 undo，与桌面一致）。成功 1。
pub fn toggle_track_solo(track: usize) -> i32 {
    toggle_override(track, false)
}

/// 新建 MIDI 轨：`cycle=true` 时在同一 port 内循环通道（B3→B16→B1），
/// 否则全部使用同一通道。成功 1。
pub fn add_tracks(count: usize, port: u8, start_channel: u8, cycle: bool) -> i32 {
    use yinhe_editor_core::NewTrackSpec;
    use yinhe_editor_core::channel_alloc;

    let count = count.clamp(1, 256);
    let alloc = if cycle {
        channel_alloc::alloc_channels_in_port(port, start_channel, count)
    } else {
        vec![(port & 0x0F, start_channel & 0x0F); count]
    };
    let specs: Vec<NewTrackSpec> = alloc
        .into_iter()
        .map(|(p, c)| NewTrackSpec {
            kind: yinhe_core::TrackKind::Midi,
            port: p,
            channel: c,
            audio_channel: None,
        })
        .collect();
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    let Some(action) = doc.add_tracks_batch(&specs) else {
        return 0;
    };
    doc.push_undo(action, "新建音轨", before);
    let model = doc.data.model.clone();
    let mask = skip_mask(&s);
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    crate::audio::set_skip_mask(mask);
    log::info!("新建音轨: {count} 条, port={port}, start={start_channel}, cycle={cycle}");
    1
}

/// 编辑音轨信息（名称/端口/通道 + 通道默认乐器）。成功 1。
#[allow(clippy::too_many_arguments)] // 对话框字段一一透传
pub fn set_track_info(
    track: usize,
    name: String,
    port: u8,
    channel: u8,
    bank: u8,
    program: u8,
    use_instrument: bool,
) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let instrument = use_instrument.then_some(yinhe_core::ChannelInstrument {
        bank_msb: bank,
        program,
    });
    let before = doc.capture_snapshot();
    let Some(action) = doc.set_track_info(track, name, port, channel, instrument) else {
        return 0;
    };
    doc.push_undo(action, "编辑音轨信息", before);
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    log::info!("音轨信息已更新: track={track}");
    1
}

/// 插入目标：kind 0=Tempo、1=PB、2=CC、3=RPN；number 为 CC 号/RPN 号。
fn insert_target(kind: i32, number: i32) -> Option<yinhe_types::automation::AutomationTarget> {
    use yinhe_types::automation::AutomationTarget;
    match kind {
        0 => Some(AutomationTarget::Tempo),
        1 => Some(AutomationTarget::PitchBend),
        2 => Some(AutomationTarget::CC {
            controller: number.clamp(0, 127) as u8,
        }),
        3 => Some(AutomationTarget::Rpn {
            parameter: number.clamp(0, 16383) as u16,
        }),
        _ => None,
    }
}

/// 长按"插入"：`value` 为显示值（PB/CC/RPN 按各自值域，Tempo 为 BPM）。
/// 成功 1。
pub fn insert_automation(kind: i32, number: i32, tick: f64, track: i32, value: f64) -> i32 {
    use yinhe_types::automation::{AutomationEvent, AutomationTarget};

    let Some(target) = insert_target(kind, number) else {
        return 0;
    };
    let is_tempo = matches!(target, AutomationTarget::Tempo);
    let track_idx = if is_tempo {
        0usize
    } else {
        track.max(0) as usize
    };
    let at_tick = tick.max(0.0).round() as u32;

    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if !is_tempo {
        let Some(t) = doc.data.model.tracks.get(track_idx) else {
            return 0;
        };
        if t.kind == yinhe_core::TrackKind::Master {
            return 0;
        }
    }
    let display = value as f32;
    let mut normalized = target.from_display_value(display);
    if is_tempo {
        normalized = normalized.clamp(1.0, 999.0);
    } else {
        normalized = normalized.clamp(0.0, 1.0);
    }
    let event = AutomationEvent {
        id: 0,
        tick: at_tick,
        value: normalized,
        shape: target.default_shape(),
    };
    let before = doc.capture_snapshot();
    let Some((_, _, action)) = doc.upsert_automation_event(track_idx, target, event) else {
        return 0;
    };
    doc.push_undo(action, "插入自动化事件", before);
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    log::info!(
        "插入自动化事件: kind={kind} number={number} tick={at_tick} track={track_idx} display={display:.1}"
    );
    1
}

/// 查询该位置当前显示值（PB/CC/RPN 原始值域；Tempo = BPM）。
/// 无历史值时返回 NaN，Dart 用目标默认值。
pub fn automation_display_value(kind: i32, number: i32, tick: f64, track: i32) -> f64 {
    use yinhe_types::automation::AutomationTarget;

    let Some(target) = insert_target(kind, number) else {
        return f64::NAN;
    };
    let Ok(s) = scene().lock() else {
        return f64::NAN;
    };
    let Some(doc) = s.document.as_ref() else {
        return f64::NAN;
    };
    let at_tick = tick.max(0.0).round() as u32;
    let normalized = if matches!(target, AutomationTarget::Tempo) {
        doc.data
            .model
            .conductor
            .tempo
            .value_at(at_tick)
            .map(|(v, _)| v)
    } else {
        doc.data
            .model
            .tracks
            .get(track.max(0) as usize)
            .and_then(|t| t.automation_lanes.iter().find(|l| l.target == target))
            .and_then(|l| l.value_at(at_tick))
            .map(|(v, _)| v)
    };
    match normalized {
        Some(v) => f64::from(target.to_display_value(v)),
        None => f64::NAN,
    }
}

/// 标尺标签（marker）JSON：`[{"tick":..,"text":".."}]`。
pub fn markers_json() -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let mut out = String::from("[");
    for (i, m) in doc.data.model.conductor.markers.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"tick\":{},\"text\":\"{}\"}}",
            m.tick,
            json_escape(&m.text)
        ));
    }
    out.push(']');
    out
}

/// 在 tick 处插入一个标签（空文本 = 显示小节位置）。成功 1。
pub fn marker_insert(tick: f64, text: String) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let t = tick.max(0.0).round() as u32;
    if !doc.insert_marker_with_undo(t, text, "undo.insert_marker_event") {
        return 0;
    }
    refresh_snapshot(&mut s);
    1
}

/// 重命名标签。成功 1。
pub fn marker_rename(tick: f64, text: String) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let t = tick.max(0.0).round() as u32;
    if !doc.set_marker_with_undo(t, t, text, "undo.rename_marker_event") {
        return 0;
    }
    refresh_snapshot(&mut s);
    1
}

/// 删除标签。成功 1。
pub fn marker_delete(tick: f64) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let t = tick.max(0.0).round() as u32;
    if !doc.delete_marker_with_undo(t, "undo.delete_marker_event") {
        return 0;
    }
    refresh_snapshot(&mut s);
    1
}

/// 移动标签：画布逻辑 x 决定目标 tick（按当前量化吸附）。成功 1。
pub fn marker_move(from: f64, x: f32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let to_tick = tick_at_x(&s, x).round() as u32;
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let from_tick = from.max(0.0).round() as u32;
    let text = doc
        .data
        .model
        .conductor
        .markers
        .iter()
        .find(|m| m.tick == from_tick)
        .map(|m| m.text.clone())
        .unwrap_or_default();
    if !doc.set_marker_with_undo(from_tick, to_tick, text, "undo.move_marker_event") {
        return 0;
    }
    refresh_snapshot(&mut s);
    1
}

/// 重命名音轨（AR 长按轨道菜单）。成功 1。/// 重命名音轨（AR 长按轨道菜单）。成功 1。
pub fn set_track_name(track: usize, name: String) -> i32 {
    use yinhe_editor_core::history::UndoAction;

    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let old = {
        let Some(doc) = s.document.as_ref() else {
            return 0;
        };
        let Some(t) = doc.data.model.tracks.get(track) else {
            return 0;
        };
        t.name.clone()
    };
    if old == name {
        return 0;
    }
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        if let Some(t) = model.tracks.get_mut(track) {
            std::sync::Arc::make_mut(t).name = name.clone();
        }
        // track 0 name = 歌曲标题（与桌面 apply 一致）。
        if track == 0 {
            model.meta.name = name.clone();
        }
    }
    doc.push_undo(
        UndoAction::TrackName {
            track_idx: track,
            old,
            new: name,
        },
        "undo.track_name",
        before,
    );
    refresh_snapshot(&mut s);
    log::info!("音轨已重命名: track={track}");
    1
}

/// AR 折叠态自动化竖线：`[{"t":track,"b":baseline,"p":[[tick,value],...]},...]`
///
/// 与桌面 `collect_folded_previews` 同语义：每条未展开 lane 以整行高竖线预览；
/// 每 lane 抽样至多 300 点（桌面按可见范围裁剪，移动端一次性下发 + 绘制时二分）。
pub fn ar_automation_json() -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let model = &doc.data.model;
    let visible = doc.edit.pr_visible();
    let mut out = String::from("[");
    let mut first = true;
    for (ti, t) in model.tracks.iter().enumerate() {
        if !visible.get(ti).copied().unwrap_or(true) {
            continue;
        }
        for lane in &t.automation_lanes {
            let Some(kind) = lane_kind(&lane.target) else {
                continue;
            };
            let n = lane.events.len();
            if n == 0 {
                continue;
            }
            if !first {
                out.push(',');
            }
            first = false;
            let baseline = if kind == 0 { 0.5 } else { 0.0 };
            out.push_str(&format!("{{\"t\":{ti},\"b\":{baseline},\"p\":["));
            let step = (n / 300).max(1);
            let mut firstp = true;
            for (i, e) in lane.events.iter().enumerate() {
                if i % step != 0 && i + 1 != n {
                    continue;
                }
                if !firstp {
                    out.push(',');
                }
                firstp = false;
                out.push_str(&format!("[{},{}]", e.tick, e.value));
            }
            out.push_str("]}");
        }
    }
    out.push(']');
    out
}

/// 滚轮/触控板：`ctrl=true` 缩放（键盘/面板区 → 副轴，其余 → 主轴），否则平移。
pub fn scroll_at(x: f32, y: f32, dx: f32, dy: f32, ctrl: bool) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let (lw, lh) = crate::logical_size();
    let total = s.total_ticks;
    let num_tracks = s.num_tracks;
    match s.view_mode {
        ViewMode::Pianoroll => {
            let view = &mut s.pr_view;
            if ctrl {
                let factor = (-dy * 0.0025).exp();
                if x < view.keyboard_width() {
                    let cross = view.cross_axis_len(lw, lh);
                    view.zoom_cross_around(y, factor, cross);
                } else {
                    view.zoom_main_around(x - view.keyboard_width(), factor);
                }
            } else {
                *view.main_scroll() -= dx + dy;
            }
            s.pr_view.clamp_scroll(lw, lh, total);
        }
        ViewMode::Arrangement => {
            let view = &mut s.arr_view;
            if ctrl {
                let factor = (-dy * 0.0025).exp();
                if x < view.base.left_panel_width {
                    view.zoom_lane_height(y, factor);
                } else {
                    view.base.zoom_around_x(x, factor);
                }
            } else {
                view.base.scroll_x -= dx + dy;
            }
            s.arr_view.clamp_scroll(lw, lh, total, num_tracks + 1);
        }
    }
    // 视图变化后选框需按内容坐标重算屏幕位置（与桌面逐帧换算一致）。
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
    1
}

/// 触控板捏合缩放：`factor > 1` 放大（键盘/面板区 → 副轴，其余 → 主轴）。
pub fn zoom_at(x: f32, y: f32, factor: f32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let (lw, lh) = crate::logical_size();
    let total = s.total_ticks;
    let num_tracks = s.num_tracks;
    let factor = factor.clamp(0.5, 2.0);
    match s.view_mode {
        ViewMode::Pianoroll => {
            let view = &mut s.pr_view;
            if x < view.keyboard_width() {
                let cross = view.cross_axis_len(lw, lh);
                view.zoom_cross_around(y, factor, cross);
            } else {
                view.zoom_main_around(x - view.keyboard_width(), factor);
            }
            s.pr_view.clamp_scroll(lw, lh, total);
        }
        ViewMode::Arrangement => {
            let view = &mut s.arr_view;
            if x < view.base.left_panel_width {
                view.zoom_lane_height(y, factor);
            } else {
                view.base.zoom_around_x(x, factor);
            }
            s.arr_view.clamp_scroll(lw, lh, total, num_tracks + 1);
        }
    }
    // 同上：缩放后选框随内容移动。
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
    1
}

/// 编辑自动化锚点（仅 PB/CC/RPN，离散）：把指定 lane 的 tick 值设为 value。
///
/// `lane_index` 为 1-based（与 `automation_json` 的输出顺序一致）。成功返回 1。
pub fn set_automation_value(track: usize, lane_index: usize, tick: u32, value: f32) -> i32 {
    use yinhe_types::automation::{AutomationEvent, SegmentShape};

    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    // 编辑前快照（undo 恢复界面状态）。
    let before_snapshot = doc.capture_snapshot();
    let model = std::sync::Arc::make_mut(&mut doc.data.model);
    let Some(t) = model.tracks.get_mut(track) else {
        return 0;
    };
    let t = std::sync::Arc::make_mut(t);
    let mut seen = 0usize;
    let mut raw_idx = 0usize;
    let mut target = None;
    for (i, lane) in t.automation_lanes.iter_mut().enumerate() {
        if lane_kind(&lane.target).is_some() {
            seen += 1;
            if seen == lane_index {
                raw_idx = i;
                target = Some(lane);
                break;
            }
        }
    }
    let Some(lane) = target else {
        return 0;
    };
    let new_event = AutomationEvent {
        id: 0,
        tick,
        value: value.clamp(0.0, 1.0),
        shape: SegmentShape::Step,
    };
    let before = lane.events.iter().find(|e| e.tick == tick).copied();
    let target_clone = lane.target.clone();
    lane.upsert(new_event);
    // 历史：单事件 upsert = 移除旧 tick 事件 + 插入新事件（一步可撤销）。
    doc.push_undo(
        yinhe_editor_core::history::UndoAction::Automation(
            yinhe_editor_core::history::AutomationDelta {
                track_idx: track,
                lane_idx: raw_idx,
                target: target_clone,
                before: before.into_iter().collect(),
                after: vec![new_event],
            },
        ),
        "Edit automation",
        before_snapshot,
    );
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    // 自动化变化：重建 CC（不中断播放）。
    crate::audio::reload_notes(model);
    log::info!("自动化编辑: track={track} lane={lane_index} tick={tick} value={value:.3}");
    1
}

/// 撤销一步（含音频与快照刷新）。成功 1。
pub fn undo() -> i32 {
    history_step(|doc| doc.undo())
}

/// 重做一步（含音频与快照刷新）。成功 1。
pub fn redo() -> i32 {
    history_step(|doc| doc.redo())
}

fn history_step(step: impl FnOnce(&mut Document) -> bool) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if !step(doc) {
        return 0;
    }
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    // 撤销/重做可能改动音符或自动化：重载 CC（不中断播放）。
    crate::audio::reload_notes(model);
    1
}

/// 设置“显示轨道”过滤（-1 = 全部）。PR/AR 音符只画该轨。成功 1。
pub fn set_display_track(track: i32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    s.display_track = if track < 0 {
        None
    } else {
        Some(track as usize)
    };
    log::info!("显示轨道: {track}");
    1
}

/// 歌曲标题（meta.name；空串 = 未设置）。
pub fn song_title() -> String {
    let Ok(s) = scene().lock() else {
        return String::new();
    };
    let Some(doc) = s.document.as_ref() else {
        return String::new();
    };
    doc.data.model.meta.name.clone()
}

/// tick → 秒（时间码预显示用；tempo 事件优先）。
pub fn seconds_at_tick(tick: f64) -> f64 {
    scene()
        .lock()
        .ok()
        .and_then(|s| s.model.clone())
        .map(|m| m.tempo_map.tick_to_seconds(tick.max(0.0) as u64))
        .unwrap_or(0.0)
}

/// 全曲音符数（顶栏标题展示用）。
pub fn note_count() -> i64 {
    scene()
        .lock()
        .ok()
        .and_then(|s| s.model.clone())
        .map(|m| m.note_count as i64)
        .unwrap_or(0)
}

/// 本工程默认 BPM / 拍号 JSON：`{"bpm":120,"num":4,"den":4}`（den = 实际分母）。
///
/// 这两个值只在 conductor 没有原生 Tempo/拍号事件时生效
/// （原生事件优先级更高，可完全代替它们）。
pub fn project_defaults_json() -> String {
    let (bpm, num, den_pow) = scene()
        .lock()
        .ok()
        .and_then(|s| s.model.clone())
        .map(|m| {
            (
                m.meta.default_bpm,
                m.meta.default_time_sig.0,
                m.meta.default_time_sig.1,
            )
        })
        .unwrap_or((120.0, 4, 2));
    let den = 1u32 << den_pow.min(8);
    format!("{{\"bpm\":{bpm},\"num\":{num},\"den\":{den}}}")
}

/// 设置本工程默认 BPM / 拍号（无原生事件时的回退）。成功 1。
pub fn set_project_defaults(bpm: f32, num: u8, den_pow: u8) -> i32 {
    let bpm = if bpm.is_finite() {
        bpm.clamp(20.0, 999.0)
    } else {
        120.0
    };
    let num = num.clamp(1, 32);
    let den_pow = den_pow.min(6);
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let model = std::sync::Arc::make_mut(&mut doc.data.model);
    if model.meta.default_bpm == bpm && model.meta.default_time_sig == (num, den_pow) {
        return 0;
    }
    model.meta.default_bpm = bpm;
    model.meta.default_time_sig = (num, den_pow);
    model.rebuild_tempo_map();
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    // tempo_map 变了：重算音符/CC 的采样时间（与 tempo 自动化编辑同机制）。
    crate::audio::reload_notes(model);
    log::info!("工程默认 BPM/拍号: {bpm:.2} BPM, {num}/{}", 1u32 << den_pow);
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
