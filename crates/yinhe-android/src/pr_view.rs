//! PR 钢琴卷帘视图：GPU cull 渲染（复用 yinhe-wgpu 管道）+ 触摸交互。
//!
//! 渲染管线：音符数据在模型加载后后台构建、一次性上传 GPU（compute cull），
//! 每帧只更新 uniforms（滚动/缩放）→ GPU cull → draw 到离屏纹理 → egui 显示。
//! 背景/网格线/键盘列由 egui 绘制（与桌面端一致的分工）。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use eframe::egui;
use yinhe_core::{Selection, YinModel};
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_theme::base::BaseColors;
use yinhe_theme::egui_colors::{Theme, derive_theme, mix};
use yinhe_types::{
    PianoRollView, TimelineViewBase, build_time_sig_segments, compute_measure_divisor,
    measure_ticks,
};
use yinhe_wgpu::{
    InstanceRenderer, NoteInstance, build_all_notes, build_key_notes, build_render_job,
};

use crate::app::Tool;

/// PR 编辑手势事件：本帧由触摸交互产生，页面层消费（写 doc + undo + 音频）。
#[derive(Debug, Clone, PartialEq)]
pub enum PrEvent {
    /// 铅笔画新音符（track 由页面层取 editing_track）。
    AddNote {
        start_tick: u32,
        end_tick: u32,
        key: u8,
    },
    /// 铅笔改音高：原音符 (track, start_tick, key) 整体平移 delta_keys。
    RetuneNote {
        track: u16,
        start_tick: u32,
        key: u8,
        delta_keys: i32,
    },
    /// 选择工具移动选中音符（绝对位移，释放时提交一次）。
    MoveNotes { delta_ticks: i64, delta_keys: i32 },
    /// 选择工具点中音符：选中单音符（按下时）。
    SelectNote { track: u16, tick: u32, key: u8 },
    /// 框选（track 范围 = editing_track 单轨）。
    SelectRect { t0: u32, t1: u32, k0: u8, k1: u8 },
    /// 橡皮框选擦除。
    EraseRect { t0: u32, t1: u32, k0: u8, k1: u8 },
    /// 橡皮单击擦除单个音符。
    EraseNote { track: u16, tick: u32, key: u8 },
    /// 单击空白：取消选择。
    ClearSelection,
}

/// PR 单指编辑手势状态机（非抓手工具、无双指时激活）：
/// 按下初始化 → 拖动中更新（egui 预览矩形）→ 释放提交事件。
#[derive(Clone, PartialEq)]
enum EditGesture {
    /// 铅笔画新音符：起点（吸附后），end_tick 跟随手指。
    PencilDraw {
        start_tick: u32,
        key: u8,
        end_tick: u32,
    },
    /// 铅笔改音高：命中的原音符，cur_key 跟随手指。
    PencilRetune {
        track: u16,
        start_tick: u32,
        key: u8,
        length: u32,
        cur_key: u8,
    },
    /// 选择框选/移动：起点 (tick, key) 浮点 + 拖动中当前点。
    /// 释放帧 interact_pos 可能已为 None，必须用拖动中保存的值，
    /// 否则选框退化成单击清除——"松手即消失"的根因。
    Marquee {
        t0: f64,
        k0: f64,
        cur_t: f64,
        cur_k: f64,
    },
    /// 选择移动音符：起点（吸附后）+ 当前点。
    MoveNotes {
        t0: f64,
        k0: f64,
        cur_t: f64,
        cur_k: f64,
    },
    /// 橡皮框选擦除：起点 + 当前点。
    EraseMarquee {
        t0: f64,
        k0: f64,
        cur_t: f64,
        cur_k: f64,
    },
}

/// 后台构建的音符实例数据（加载后一次性构建 + 上传）。
struct NoteBuildResult {
    notes: Vec<NoteInstance>,
    offsets: [u32; 129],
}

/// 顶部小节标尺高度（px）：内容区整体下移该距离给标尺让位。
/// 手机屏幕紧张，比桌面端（~28px）更矮。
const RULER_H: f32 = 16.0;

/// PR 视图：视口状态 + GPU 渲染 + 触摸交互。
pub struct PrView {
    wgpu_state: Arc<eframe::egui_wgpu::RenderState>,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    texture_id: egui::TextureId,
    renderer: Option<InstanceRenderer>,
    /// uniforms/视口逻辑尺寸（逻辑像素，egui 坐标单位）。
    width: u32,
    height: u32,
    /// 离屏纹理物理尺寸（逻辑尺寸 × pixels_per_point）。纹理按物理像素
    /// 创建，否则高分屏上音符发虚。
    tex_w: u32,
    tex_h: u32,
    /// 当前像素密度（每帧从 egui 读取）。
    ppp: f32,
    view: PianoRollView,
    model: Option<Arc<YinModel>>,
    notes_build: Option<Arc<Mutex<Option<NoteBuildResult>>>>,
    notes_uploaded: bool,
    status: String,
    keyboard_w: f32,
    /// 首次渲染时是否已完成初始视口定位（key 60 居中）。
    view_initialized: bool,
    /// 播放光标 tick（None = 不显示）。由 lib.rs 每帧从音频位置换算后设置。
    cursor_tick: Option<f64>,
    /// 上一帧触点数：检测第二指落下的那一帧（忽略该帧缩放跳变）。
    prev_touches: u32,
    /// 上次上传到 GPU cull 的轨道可见性掩码（比较检测变化，变化才上传）。
    last_mask: Vec<bool>,
    /// 当前编辑手势（非抓手工具单指按下时激活）。
    gesture: Option<EditGesture>,
    /// 当前选区（doc.edit.selected 的引用副本，渲染高亮 + 移动预览用）。
    selected: Selection,
    /// 主题色（与桌面端同源：BaseColors 7 色派生）。
    theme: Theme,
}

impl PrView {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let wgpu_state: Arc<eframe::egui_wgpu::RenderState> = cc
            .wgpu_render_state
            .clone()
            .expect("wgpu backend required")
            .into();
        let (texture, texture_view, texture_id) = create_target(
            &wgpu_state.device,
            &mut wgpu_state.renderer.write(),
            wgpu_state.target_format,
            1,
            1,
        );
        Self {
            wgpu_state,
            texture,
            texture_view,
            texture_id,
            renderer: None,
            width: 1,
            height: 1,
            tex_w: 1,
            tex_h: 1,
            ppp: 1.0,
            view: PianoRollView {
                base: TimelineViewBase {
                    pixels_per_tick: 0.05,
                    scroll_x: 0.0,
                    scroll_y: 0.0,
                    left_panel_width: 0.0,
                    dirty: true,
                    track_panel_row_height: 0.0,
                    track_panel_scroll_y: 0.0,
                },
                key_height: 12.0,
                viewport_h: 0.0,
            },
            model: None,
            notes_build: None,
            notes_uploaded: false,
            status: String::new(),
            keyboard_w: 0.0,
            view_initialized: false,
            cursor_tick: None,
            prev_touches: 0,
            last_mask: Vec::new(),
            gesture: None,
            selected: Selection::default(),
            theme: derive_theme(BaseColors::DARK),
        }
    }

    /// 设置模型并初始化视口（全曲可见）。
    pub fn set_model(&mut self, model: Arc<YinModel>) {
        // 初始缩放：全曲宽约两个屏幕，用户再捏合放大。
        let tick_length = model.tempo_map.tick_length.max(1) as f32;
        let ppu = (1280.0 * 2.0 / tick_length).clamp(0.0005, 8.0);
        self.view.base.pixels_per_tick = ppu;
        self.view.base.scroll_x = 0.0;
        self.view.base.scroll_y = 0.0;
        self.view.base.dirty = true;
        self.model = Some(model);
        self.notes_build = None;
        self.notes_uploaded = false;
        self.last_mask.clear();
        self.gesture = None;
        self.selected = Selection::default();
        self.status = "正在构建音符数据...".to_string();
    }

    /// 设置播放光标位置（tick），None 隐藏。由 lib.rs 每帧从音频位置换算后调用。
    pub fn set_cursor(&mut self, tick: Option<f64>) {
        self.cursor_tick = tick;
    }

    /// 播放跟随：水平滚动让光标位于内容区中央（lib.rs 在跟随模式开启时每帧调用）。
    pub fn follow_cursor(&mut self) {
        let Some(tick) = self.cursor_tick else {
            return;
        };
        let content_w = (self.width as f32 - self.keyboard_w).max(1.0);
        let target = tick as f32 * self.view.base.pixels_per_tick - content_w / 2.0;
        self.view.base.scroll_x = target.max(0.0);
        self.view.base.dirty = true;
    }

    /// 主 UI 入口：背景 + 键盘列 + 网格 + GPU 音符层 + 触摸交互。
    /// safe 为安全区 insets（逻辑点）：[left, top, right, bottom]。
    /// track_visible 为轨道显隐（doc.edit.track_visible），editing_track 为
    /// 当前编辑轨（doc.edit.editing_track）——编辑轨强制可见。
    /// `tool` 为当前工具（抓手=单指滚动，其他=编辑手势）；`selected` 为
    /// 当前选区（doc.edit.selected，渲染高亮 + 移动预览）。`quantize` 为
    /// 当前量化（doc.edit.quantize_pianoroll，与 AR 的 quantize_arrange 独立）。
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        safe: [f32; 4],
        track_visible: &[bool],
        editing_track: Option<u16>,
        hand_scroll: bool,
        tool: Tool,
        selected: &Selection,
        quantize: QuantizePreset,
    ) -> Vec<PrEvent> {
        let mut events = Vec::new();
        let full = ui.available_rect_before_wrap();
        let painter = ui.painter();
        // 背景铺满整个视口（延伸到挖孔/刘海后面，视觉融合）；
        // 内容区（纹理/键盘/标尺）整体避开安全区，挖孔区域只显示背景色。
        painter.rect_filled(full, 0.0, self.theme.app_bg);
        let rect = egui::Rect::from_min_max(
            full.min + egui::vec2(safe[0], safe[1]),
            full.max - egui::vec2(safe[2], safe[3]),
        );
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return events;
        }

        // 顶部小节标尺带：高度充足时内容区整体下移 24px 让位，否则不画标尺。
        let ruler_h = if rect.height() > 200.0 { RULER_H } else { 0.0 };

        let Some(model) = self.model.clone() else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "未加载 MIDI",
                egui::FontId::proportional(18.0),
                self.theme.text_muted,
            );
            return events;
        };

        self.keyboard_w = 64.0;
        self.view.base.left_panel_width = self.keyboard_w;
        self.view.viewport_h = rect.height();
        // 像素密度：纹理按物理像素创建（高分屏不发虚）。
        self.ppp = ui.ctx().pixels_per_point().max(0.25);
        // 首次渲染：纵向定位到中央音区（key 60），否则 scroll_y=0 显示最高音区。
        // 与 GPU 音符层同一参考系：key 127 在顶，key 60 行中心 = 128kh - 60.5kh。
        if !self.view_initialized {
            let kh = self.view.key_height;
            self.view.base.scroll_y = ((128.0 - 60.5) * kh - rect.height() / 2.0).max(0.0);
            self.view_initialized = true;
        }
        self.ensure_texture_size(rect);
        // egui 层内容区：键盘列与内容区都从标尺带下方开始，与音符层（纹理
        // 1:1 贴图、顶部让位）保持对齐。手势坐标换算与预览绘制都用它。
        let kb_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, rect.min.y + ruler_h),
            egui::pos2(rect.min.x + self.keyboard_w, rect.max.y),
        );
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + self.keyboard_w, rect.min.y + ruler_h),
            rect.max,
        );
        self.handle_touch(ui, rect, hand_scroll);
        self.ensure_renderer();
        self.ensure_notes(&model);
        self.selected = selected.clone();
        if !hand_scroll {
            self.handle_edit_gesture(
                ui,
                rect,
                content_rect,
                &model,
                tool,
                editing_track,
                quantize,
                model.meta.ppq,
                &mut events,
            );
        }
        // 编辑后增量同步：比较 per-key revision，变化的 key 重建并上传（
        // build_key_notes 只扫命中桶，1 亿音符工程编辑也只重建受影响桶）。
        self.sync_edited_keys();

        // 轨道显隐掩码：PR 可见性 = track_visible，且编辑轨强制可见。
        // 与上次上传比较，变化才上传（upload_track_mask 会强制 cull 重跑
        // dispatch，每帧上传会浪费 GPU；重新可见无需重建音符数据）。
        if let Some(renderer) = &mut self.renderer {
            let pr_visible: Vec<bool> = (0..track_visible.len())
                .map(|i| track_visible[i] || editing_track == Some(i as u16))
                .collect();
            if pr_visible != self.last_mask {
                renderer.upload_track_mask(&pr_visible);
                self.last_mask = pr_visible;
            }
        }

        if let Some(renderer) = &mut self.renderer {
            let track_colors = crate::track_colors_for(&model);
            let job = build_render_job(
                self.width,
                self.height,
                &self.view,
                &self.selected,
                &track_colors,
                0,
                0.0,
                true,
            );
            renderer.upload_uniforms(job.uniforms);
            renderer.upload_track_colors(&job.track_colors);
            renderer.upload_selection(&job.selection);
            let mut encoder = self
                .wgpu_state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            // draw 的 pass 自带透明 clear，音符绘制在这里完成。
            // viewport 必须用纹理物理尺寸。
            renderer.draw(&mut encoder, &self.texture_view, self.tex_w, self.tex_h);
            self.wgpu_state.queue.submit([encoder.finish()]);
        }

        // egui 层绘制（键盘列/标尺/网格/纹理贴图）。
        self.draw_scale_bands(ui, content_rect);
        self.draw_octave_lines(ui, content_rect);
        self.draw_keyboard(ui, kb_rect);
        self.draw_grid(ui, content_rect);
        // 离屏纹理覆盖整个 rect（含键盘列，与桌面端一致）：shader 的音符
        // 像素坐标从 keyboard_width 起步，NDC 用总宽做分母，纹理必须同宽，
        // 否则音符整体右移 keyboard_width 像素且右端溢出被裁（安卓看不到
        // 音符的根因）。键盘列是纹理的透明区，由 egui 键盘层透出。
        // 顶部 ruler_h 像素让给标尺：贴图目标与 UV 同步裁掉该段，保持 1:1。
        // UV 用物理像素（纹理是物理尺寸，ruler_h 是逻辑像素）。
        painter.image(
            self.texture_id,
            egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.min.y + ruler_h), rect.max),
            egui::Rect::from_min_max(
                egui::pos2(0.0, (ruler_h * self.ppp) / self.tex_h.max(1) as f32),
                egui::pos2(1.0, 1.0),
            ),
            egui::Color32::WHITE,
        );
        self.draw_ruler(ui, rect, content_rect, ruler_h);
        self.draw_cursor(ui, rect, ruler_h);
        // 编辑手势预览（画音符/选框），叠在音符层之上。
        self.draw_gesture_preview(ui, rect, content_rect);

        let notes: u64 = model.track_note_count.iter().sum();
        let status = format!(
            "{} | {notes} 音符 | {:.3} px/tick | 双指缩放/拖动",
            self.status, self.view.base.pixels_per_tick,
        );
        painter.text(
            egui::pos2(rect.min.x + 8.0, rect.min.y + ruler_h + 8.0),
            egui::Align2::LEFT_TOP,
            status,
            egui::FontId::proportional(13.0),
            self.theme.text_label,
        );
        events
    }

    /// 触摸手势：双指永远导航（平移+缩放）；单指滚动仅抓手工具。
    /// 缩放判定阈值 2% + 第二指落帧忽略，防误缩放。
    fn handle_touch(&mut self, ui: &egui::Ui, rect: egui::Rect, hand_scroll: bool) {
        let (zoom, pan, touches, pointer_delta, pointer_down, touch_center) = ui.ctx().input(|i| {
            let mt = i.multi_touch();
            (
                mt.map(|m| m.zoom_delta),
                mt.map(|m| m.translation_delta),
                mt.map(|m| m.num_touches).unwrap_or(0),
                i.pointer.delta(),
                i.pointer.primary_down(),
                mt.map(|m| m.center_pos),
            )
        });
        // 第二指刚落下那一帧 zoom_delta 有距离跳变，忽略缩放（防闪缩）。
        let fresh_pinch = touches >= 2 && self.prev_touches < 2;
        self.prev_touches = touches as u32;
        // 双指导航优先：取消编辑手势，避免双指滚动/缩放时误编辑。
        if touches >= 2 {
            self.gesture = None;
        }
        let base = &mut self.view.base;
        if let Some(zoom) = zoom
            && (zoom - 1.0).abs() > 0.02
            && !fresh_pinch
        {
            if touch_center.is_some_and(|c| c.x < rect.min.x + self.keyboard_w) {
                // 键盘列上捏合 → 垂直缩放 key_height。锚点 = 双指中心对应的
                // key 浮点位置，缩放后该 key 仍停留在手指处（同 shader 参考系）。
                // 下限 = 128 键恰好铺满视口（不允许比视口更小）。
                let y0 = touch_center.expect("checked above").y;
                let kh = self.view.key_height;
                let bottom = 128.0 * kh - base.scroll_y;
                let anchor_key = (bottom - (y0 - rect.min.y)) / kh;
                let new_kh = (kh * zoom).clamp(rect.height() / 128.0, 48.0);
                let new_bottom = (y0 - rect.min.y) + anchor_key * new_kh;
                let max_scroll = (128.0 * new_kh - rect.height()).max(0.0);
                base.scroll_y = (128.0 * new_kh - new_bottom).clamp(0.0, max_scroll);
                self.view.key_height = new_kh;
                base.dirty = true;
            } else {
                // 内容区捏合 → 水平缩放。锚点 = 手指（双指中心）在内容区内
                // 的位置，缩放后该 tick 停在手指处；无手指信息时取内容区中点。
                let content_w = rect.width() - self.keyboard_w;
                let cx = touch_center
                    .map(|c| (c.x - rect.min.x - self.keyboard_w).clamp(0.0, content_w))
                    .unwrap_or(content_w / 2.0);
                let center_tick = (base.scroll_x + cx) / base.pixels_per_tick.max(1e-6);
                base.pixels_per_tick = (base.pixels_per_tick * zoom).clamp(0.0005, 8.0);
                base.scroll_x = (center_tick * base.pixels_per_tick - cx).max(0.0);
                base.dirty = true;
            }
        }
        // 垂直滚动上限：128 键总高（128*kh）不能小于视口高，滚到底即全部可见。
        let max_scroll = (128.0 * self.view.key_height - rect.height()).max(0.0);
        if touches >= 2 {
            if let Some(pan) = pan {
                base.scroll_x = (base.scroll_x - pan.x).max(0.0);
                base.scroll_y = (base.scroll_y - pan.y).clamp(0.0, max_scroll);
                base.dirty = true;
            }
        } else if hand_scroll && pointer_down {
            base.scroll_x = (base.scroll_x - pointer_delta.x).max(0.0);
            base.scroll_y = (base.scroll_y - pointer_delta.y).clamp(0.0, max_scroll);
            base.dirty = true;
        }
    }

    /// 单指编辑手势状态机（非抓手工具时调用）：
    /// 按下初始化 → 拖动中更新（egui 预览矩形）→ 释放提交事件。
    /// 双指（导航）期间不进入；`editing_track` 为编辑目标轨。
    #[allow(clippy::too_many_arguments)]
    fn handle_edit_gesture(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        content_rect: egui::Rect,
        model: &Arc<YinModel>,
        tool: Tool,
        editing_track: Option<u16>,
        quantize: QuantizePreset,
        ppq: u32,
        events: &mut Vec<PrEvent>,
    ) {
        let (touches, pressed, released, pos, down) = ui.ctx().input(|i| {
            let mt = i.multi_touch();
            (
                mt.map(|m| m.num_touches).unwrap_or(0),
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
                i.pointer.primary_down(),
            )
        });
        if touches >= 2 {
            self.gesture = None;
            return;
        }
        let Some(editing) = editing_track else {
            return;
        };
        // 位置 → 吸附后的 (tick, key)。坐标参考系与 GPU 音符层一致：
        // x 相对 rect.min（x_to_tick 内部减键盘列宽），y 相对内容区顶。
        // view 用参数传入：拖动中 auto_scroll 需要 &mut self，闭包不能捕获 self。
        let local = |view: &PianoRollView, pos: egui::Pos2| {
            let raw_tick = view.x_to_tick(pos.x - rect.min.x);
            let tick = quantize.snap_tick(raw_tick, ppq).max(0.0) as u32;
            let key = view.y_to_key(pos.y - content_rect.min.y);
            (tick, key)
        };

        if pressed
            && let Some(pos) = pos
            && self.gesture.is_none()
        {
            let (tick, key) = local(&self.view, pos);
            match tool {
                Tool::Pencil => {
                    if let Some((nt, ne, nk)) = self.hit_note(model, editing, tick, key) {
                        self.gesture = Some(EditGesture::PencilRetune {
                            track: editing,
                            start_tick: nt,
                            key: nk,
                            length: ne - nt,
                            cur_key: key,
                        });
                    } else {
                        // 默认长度 = 一个量化网格；拖动拉长，单击画默认长度。
                        let interval = quantize.tick_interval(ppq).max(1);
                        self.gesture = Some(EditGesture::PencilDraw {
                            start_tick: tick,
                            key,
                            end_tick: tick.saturating_add(interval),
                        });
                    }
                }
                Tool::Select => {
                    if self.hit_note(model, editing, tick, key).is_some() {
                        events.push(PrEvent::SelectNote {
                            track: editing,
                            tick,
                            key,
                        });
                        self.gesture = Some(EditGesture::MoveNotes {
                            t0: tick as f64,
                            k0: key as f64,
                            cur_t: tick as f64,
                            cur_k: key as f64,
                        });
                    } else {
                        self.gesture = Some(EditGesture::Marquee {
                            t0: tick as f64,
                            k0: key as f64,
                            cur_t: tick as f64,
                            cur_k: key as f64,
                        });
                    }
                }
                Tool::Eraser => {
                    self.gesture = Some(EditGesture::EraseMarquee {
                        t0: tick as f64,
                        k0: key as f64,
                        cur_t: tick as f64,
                        cur_k: key as f64,
                    });
                }
                Tool::Hand => {}
            }
        }

        // 拖动中：先边缘自动滚动视口，再更新手势（tick/key 跟随手指）。
        if down && let Some(pos) = pos {
            // 拖到内容区边缘 20px 内自动滚动（桌面端同款），选区/音符坐标
            // 是音乐坐标（tick/key），滚动后预览自动跟随。
            self.auto_scroll(ui, content_rect, pos);
            if let Some(g) = &mut self.gesture {
                let (tick, key) = local(&self.view, pos);
                match g {
                    EditGesture::PencilDraw {
                        start_tick,
                        end_tick,
                        ..
                    } => {
                        *end_tick = tick.max(start_tick.saturating_add(1));
                    }
                    EditGesture::PencilRetune { cur_key, .. } => {
                        *cur_key = key;
                    }
                    EditGesture::Marquee { cur_t, cur_k, .. }
                    | EditGesture::MoveNotes { cur_t, cur_k, .. }
                    | EditGesture::EraseMarquee { cur_t, cur_k, .. } => {
                        *cur_t = tick as f64;
                        *cur_k = key as f64;
                    }
                }
            }
        }

        // 释放：提交事件（一次 undo entry 的原材料）。
        // 注意：不能依赖释放帧的 interact_pos（可能已为 None），
        // 一律用拖动中保存在手势里的当前点，否则选框退化成单击清除。
        if released && let Some(g) = self.gesture.take() {
            match g {
                EditGesture::PencilDraw {
                    start_tick,
                    key,
                    end_tick,
                } => {
                    events.push(PrEvent::AddNote {
                        start_tick,
                        end_tick: end_tick.max(start_tick + 1),
                        key,
                    });
                }
                EditGesture::PencilRetune {
                    track,
                    start_tick,
                    key,
                    cur_key,
                    ..
                } => {
                    let dk = cur_key as i32 - key as i32;
                    if dk != 0 {
                        events.push(PrEvent::RetuneNote {
                            track,
                            start_tick,
                            key,
                            delta_keys: dk,
                        });
                    }
                }
                EditGesture::MoveNotes {
                    t0,
                    k0,
                    cur_t,
                    cur_k,
                    ..
                } => {
                    let dt = cur_t as i64 - t0 as i64;
                    let dk = cur_k as i64 - k0 as i64;
                    if dt != 0 || dk != 0 {
                        events.push(PrEvent::MoveNotes {
                            delta_ticks: dt,
                            delta_keys: dk as i32,
                        });
                    }
                    // dt==0 && dk==0：纯单击选中（SelectNote 已选，无 undo）。
                }
                EditGesture::Marquee {
                    t0,
                    k0,
                    cur_t,
                    cur_k,
                    ..
                } => {
                    if cur_t != t0 || cur_k != k0 {
                        let (a, b) = (t0.min(cur_t), t0.max(cur_t));
                        let (ka, kb) = (k0.min(cur_k), k0.max(cur_k));
                        // 释放帧直接更新本地渲染选区：事件处理在绘制后才写 doc，
                        // 否则这一帧预览消失、持久选框未到，会闪一帧空白。
                        let mut sel = Selection::default();
                        sel.add_rect_track(
                            a as u32,
                            b as u32 + 1,
                            ka as u8,
                            kb as u8,
                            editing,
                            editing,
                        );
                        self.selected = sel;
                        events.push(PrEvent::SelectRect {
                            t0: a as u32,
                            t1: b as u32 + 1,
                            k0: ka as u8,
                            k1: kb as u8,
                        });
                    } else {
                        // 单击空白：清选区（同步本地，防闪烁）。
                        self.selected = Selection::default();
                        events.push(PrEvent::ClearSelection);
                    }
                }
                EditGesture::EraseMarquee {
                    t0,
                    k0,
                    cur_t,
                    cur_k,
                    ..
                } => {
                    if cur_t != t0 || cur_k != k0 {
                        let (a, b) = (t0.min(cur_t), t0.max(cur_t));
                        let (ka, kb) = (k0.min(cur_k), k0.max(cur_k));
                        events.push(PrEvent::EraseRect {
                            t0: a as u32,
                            t1: b as u32 + 1,
                            k0: ka as u8,
                            k1: kb as u8,
                        });
                    } else {
                        events.push(PrEvent::EraseNote {
                            track: editing,
                            tick: cur_t as u32,
                            key: cur_k as u8,
                        });
                    }
                }
            }
        }
    }

    /// 命中检测：点击网格点处该 key 桶中覆盖点击 tick 的同轨音符
    ///（黑乐谱同 tick 多音符时取最上层=最后一个），返回 (start, end, key)。
    fn hit_note(&self, model: &YinModel, track: u16, tick: u32, key: u8) -> Option<(u32, u32, u8)> {
        let lo = tick.saturating_sub(2);
        let hi = tick.saturating_add(3);
        model.notes[key as usize]
            .range(lo, hi)
            .filter(|n| n.track == track && n.start_tick <= tick && n.end_tick > tick)
            .map(|n| (n.start_tick, n.end_tick, key))
            .last()
    }

    /// 拖动中边缘自动滚动视口（桌面端 auto_scroll_on_drag 同款，触发区
    /// 放宽到 48px：手机屏幕边缘难精确按住，太窄的触发区拉不到）。
    fn auto_scroll(&mut self, ui: &egui::Ui, content_rect: egui::Rect, pos: egui::Pos2) {
        const MARGIN: f32 = 48.0;
        const BASE_SPEED: f32 = 15.0;
        let dt = ui.input(|i| i.unstable_dt);
        let mut dx = 0.0f32;
        let mut dy = 0.0f32;
        if pos.x < content_rect.min.x + MARGIN {
            dx = -(content_rect.min.x + MARGIN - pos.x) * BASE_SPEED * dt;
        } else if pos.x > content_rect.max.x - MARGIN {
            dx = (pos.x - (content_rect.max.x - MARGIN)) * BASE_SPEED * dt;
        }
        if pos.y < content_rect.min.y + MARGIN {
            dy = -(content_rect.min.y + MARGIN - pos.y) * BASE_SPEED * dt;
        } else if pos.y > content_rect.max.y - MARGIN {
            dy = (pos.y - (content_rect.max.y - MARGIN)) * BASE_SPEED * dt;
        }
        if dx != 0.0 || dy != 0.0 {
            let base = &mut self.view.base;
            base.scroll_x = (base.scroll_x + dx).max(0.0);
            base.scroll_y = (base.scroll_y + dy).max(0.0);
            let total_ticks = self
                .model
                .as_ref()
                .map_or(0.0, |m| m.tempo_map.tick_length as f64);
            base.clamp_scroll_x(content_rect.width(), total_ticks);
            base.dirty = true;
            ui.ctx().request_repaint();
        }
    }

    /// 编辑后增量同步：比较 per-key revision，变化的 key 重建 NoteInstance 并
    /// 增量上传 GPU cull（只扫命中桶，1 亿音符工程编辑也只重建受影响桶）。
    fn sync_edited_keys(&mut self) {
        let Some(model) = &self.model else {
            return;
        };
        if !self.notes_uploaded {
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let uploaded = *renderer.uploaded_key_revisions();
        let n = model.tracks.len();
        for key in 0..128u8 {
            let rev = model.note_revisions[key as usize];
            if uploaded[key as usize] == rev {
                continue;
            }
            let notes = build_key_notes(model.as_ref(), key, &HashSet::new(), &vec![true; n]);
            if !renderer.try_incremental_key_upload(key, &notes, rev) {
                // 该 key 从未上传（异常路径）：退化为全量重建。
                self.notes_build = None;
                self.notes_uploaded = false;
                return;
            }
        }
    }

    /// 编辑手势预览：铅笔音符/改音高/移动选区/选框，叠在音符层之上。
    /// 选框/移动预览的 tick 与提交一致（按当前量化吸附），避免"预览没吸附、
    /// 松手跳一格"的错位感。
    fn draw_gesture_preview(&self, ui: &egui::Ui, rect: egui::Rect, content_rect: egui::Rect) {
        let Some(g) = &self.gesture else {
            return;
        };
        let painter = ui.painter();
        let preview_color = self.theme.accent_active.gamma_multiply(0.45);
        let preview_stroke = egui::Stroke::new(1.5, self.theme.accent_active);
        // 音符矩形：tick 区间 + key 行。坐标与 GPU 音符层一致。
        let note_rect = |painter: &egui::Painter, t0: f64, t1: f64, key: u8| {
            let x0 = rect.min.x + self.view.tick_to_x(t0);
            let x1 = rect.min.x + self.view.tick_to_x(t1);
            let y0 = content_rect.min.y + self.view.key_to_y(key);
            let y1 = content_rect.min.y + self.view.key_to_y(key + 1);
            let r = egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
                .intersect(content_rect);
            painter.rect_filled(r, 2.0, preview_color);
            painter.rect_stroke(r, 2.0, preview_stroke, egui::StrokeKind::Inside);
        };
        match g {
            EditGesture::PencilDraw {
                start_tick,
                key,
                end_tick,
            } => {
                note_rect(
                    painter,
                    *start_tick as f64,
                    (*end_tick).max(*start_tick + 1) as f64,
                    *key,
                );
            }
            EditGesture::PencilRetune {
                start_tick,
                length,
                cur_key,
                ..
            } => {
                note_rect(
                    painter,
                    *start_tick as f64,
                    (*start_tick + *length) as f64,
                    *cur_key,
                );
            }
            EditGesture::Marquee {
                t0,
                k0,
                cur_t,
                cur_k,
                ..
            }
            | EditGesture::EraseMarquee {
                t0,
                k0,
                cur_t,
                cur_k,
                ..
            } => {
                let x0 = rect.min.x + self.view.tick_to_x(*t0);
                let x1 = rect.min.x + self.view.tick_to_x(*cur_t);
                let y0 = content_rect.min.y + self.view.key_to_y((*k0).min(*cur_k) as u8);
                let y1 = content_rect.min.y + self.view.key_to_y((*k0).max(*cur_k) as u8 + 1);
                let r = egui::Rect::from_min_max(
                    egui::pos2(x0.min(x1), y0.min(y1)),
                    egui::pos2(x0.max(x1), y0.max(y1)),
                )
                .intersect(content_rect);
                painter.rect_filled(r, 1.0, preview_color);
                painter.rect_stroke(r, 1.0, preview_stroke, egui::StrokeKind::Inside);
            }
            EditGesture::MoveNotes {
                t0,
                k0,
                cur_t,
                cur_k,
                ..
            } => {
                // 拖动中把选中音符的矩形偏移画预览（选区矩形偏移显示）。
                let dt = *cur_t - *t0;
                let dk = *cur_k - *k0;
                for &(ts, te, kl, kh, _, _) in &self.selected.rects {
                    let x0 = rect.min.x + self.view.tick_to_x(ts as f64 + dt);
                    let x1 = rect.min.x + self.view.tick_to_x(te as f64 + dt);
                    let y0 =
                        content_rect.min.y + self.view.key_to_y((kl as f64 + dk).max(0.0) as u8);
                    let y1 = content_rect.min.y
                        + self.view.key_to_y((kh as f64 + dk).max(0.0) as u8 + 1);
                    let r = egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
                        .intersect(content_rect);
                    painter.rect_filled(r, 2.0, preview_color);
                    painter.rect_stroke(r, 2.0, preview_stroke, egui::StrokeKind::Inside);
                }
            }
        }
    }

    /// 惰性初始化 InstanceRenderer（GPU 管道：compute cull + draw）。
    fn ensure_renderer(&mut self) {
        if self.renderer.is_none() {
            // 注册 wgpu 错误回调：pipeline/bind group 创建失败在 wgpu 30 是静默的
            //（错误进 uncaptured error），安卓 stderr 不可见，必须显式打到 logcat。
            let device = self.wgpu_state.device.clone();
            device.on_uncaptured_error(Arc::new(|err| {
                log::error!("wgpu uncaptured error: {err}");
            }));
            let queue = self.wgpu_state.queue.clone();
            let format = self.wgpu_state.target_format;
            log::info!("pr_view: InstanceRenderer 初始化，target_format={format:?}");
            log::info!(
                "pr_view: 设备 features INDIRECT_FIRST_INSTANCE={}",
                device
                    .features()
                    .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
            );
            self.renderer = Some(InstanceRenderer::new(device, queue, format));
            log::info!("pr_view: InstanceRenderer 初始化完成");
        }
    }

    /// 音符数据：后台线程构建（build_all_notes 是 CPU 密集），完成后上传 GPU。
    fn ensure_notes(&mut self, model: &Arc<YinModel>) {
        if self.notes_uploaded {
            return;
        }
        if self.notes_build.is_none() {
            let model = model.clone();
            let state: Arc<Mutex<Option<NoteBuildResult>>> = Arc::new(Mutex::new(None));
            let state2 = state.clone();
            log::info!("pr_view: 启动音符构建线程");
            std::thread::Builder::new()
                .name("yinhe-note-build".into())
                .spawn(move || {
                    // catch_unwind：安卓上 Rust panic 的 stderr 不可见（不进 logcat），
                    // 不捕获的话 UI 永远停在"正在构建"且无任何线索。
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let track_visible = vec![true; model.tracks.len()];
                        let (notes, offsets) =
                            build_all_notes(model.as_ref(), &HashSet::new(), &track_visible);
                        NoteBuildResult { notes, offsets }
                    }));
                    match result {
                        Ok(r) => {
                            log::info!("pr_view: 音符构建完成 {} 实例", r.notes.len());
                            *state2.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                        }
                        Err(e) => {
                            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                                (*s).to_string()
                            } else if let Some(s) = e.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "unknown panic".to_string()
                            };
                            log::error!("pr_view: build_all_notes panic: {msg}");
                            // 用空结果标记失败（None 表示仍在构建）
                            *state2.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(NoteBuildResult {
                                    notes: Vec::new(),
                                    offsets: [0; 129],
                                });
                        }
                    }
                })
                .expect("failed to spawn note build thread");
            self.notes_build = Some(state);
        }
        if let Some(state) = &self.notes_build {
            let result = state.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(result) = result
                && let Some(renderer) = &mut self.renderer
            {
                if result.notes.is_empty() {
                    self.status = "音符构建失败（见 logcat）".to_string();
                    self.notes_uploaded = true; // 不再重试
                    return;
                }
                renderer.upload_all_notes_for_cull(&result.notes, &result.offsets, &[0u64; 128]);
                self.notes_uploaded = true;
                let notes: u64 = model.track_note_count.iter().sum();
                self.status = format!("{notes} 音符已上传 GPU");
            }
        }
    }

    /// 离屏纹理尺寸跟随可用区域（双向重建）。
    /// 纹理 = 逻辑尺寸 × pixels_per_point（物理像素，高分屏清晰）；
    /// uniforms 与滚动仍是逻辑坐标（shader 里 NDC 用 u.width 做分母，
    /// 只要分子分母同单位比例就正确，纹理物理尺寸只决定采样分辨率）。
    fn ensure_texture_size(&mut self, rect: egui::Rect) {
        let w = rect.width().max(1.0) as u32;
        let h = rect.height().max(1.0) as u32;
        let tex_w = (rect.width() * self.ppp).round().max(1.0) as u32;
        let tex_h = (rect.height() * self.ppp).round().max(1.0) as u32;
        if w == self.width && h == self.height && tex_w == self.tex_w && tex_h == self.tex_h {
            return;
        }
        let (texture, texture_view, texture_id) = create_target(
            &self.wgpu_state.device,
            &mut self.wgpu_state.renderer.write(),
            self.wgpu_state.target_format,
            tex_w,
            tex_h,
        );
        self.texture = texture;
        self.texture_view = texture_view;
        self.texture_id = texture_id;
        self.width = w;
        self.height = h;
        self.tex_w = tex_w;
        self.tex_h = tex_h;
    }

    /// 键盘列：可见键范围内的黑白键。
    /// 纵向参考系与 GPU 音符层一致（key 127 在顶，piano 惯例高音在上）：
    /// 之前用 key 0 在顶的公式导致键盘列上下颠倒且与音符层错位。
    fn draw_keyboard(&self, ui: &egui::Ui, rect: egui::Rect) {
        let kh = self.view.key_height;
        let scroll_y = self.view.base.scroll_y;
        let bottom = 128.0 * kh - scroll_y;
        let top_key = ((bottom / kh).ceil() as i32 - 1).clamp(0, 127);
        let bottom_key = (((bottom - rect.height()) / kh).ceil() as i32 - 1).clamp(0, 127);
        let (key_lo, key_hi) = (bottom_key.min(top_key), bottom_key.max(top_key));
        let painter = ui.painter();
        let is_black = |k: i32| matches!(k % 12, 1 | 3 | 6 | 8 | 10);
        for key in key_lo..=key_hi {
            let y = rect.min.y + bottom - (key as f32 + 1.0) * kh;
            let color = if is_black(key) {
                self.theme.track_bg
            } else {
                self.theme.control_bg
            };
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y + kh)),
                0.0,
                color,
            );
        }
        painter.line_segment(
            [
                egui::pos2(rect.max.x, rect.min.y),
                egui::pos2(rect.max.x, rect.max.y),
            ],
            egui::Stroke::new(1.0, self.theme.line_fg),
        );
    }

    /// 横向背景色带：有调号时按当前生效调式着色（调内音浅色、根音更亮），
    /// 无调号时退化为白键行浅色——与桌面端 bg::paint 同一策略，辅助定位
    /// 音高与调性。y 与 GPU 音符层同参考系（key 越大越靠上）。
    fn draw_scale_bands(&self, ui: &egui::Ui, rect: egui::Rect) {
        let kh = self.view.key_height;
        let scroll_y = self.view.base.scroll_y;
        let bottom = 128.0 * kh - scroll_y;
        let top_key = ((bottom / kh).ceil() as i32 - 1).clamp(0, 127);
        let bottom_key = (((bottom - rect.height()) / kh).ceil() as i32 - 1).clamp(0, 127);
        let (key_lo, key_hi) = (bottom_key.min(top_key), bottom_key.max(top_key));
        let painter = ui.painter();
        let band = |painter: &egui::Painter, key: i32, color: egui::Color32| {
            let y = rect.min.y + bottom - (key as f32 + 1.0) * kh;
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y + kh)),
                0.0,
                color,
            );
        };

        // 当前生效调号：视口起点之前最后一个调号事件（无则取第一个）。
        let scroll_tick = self.view.base.scroll_x / self.view.base.pixels_per_tick.max(1e-6);
        let scroll_tick = scroll_tick as f64;
        let key_sig = self
            .model
            .as_ref()
            .map(|m| m.conductor.key_sig.as_slice())
            .unwrap_or(&[]);
        let eff = key_sig
            .iter()
            .rev()
            .find(|e| e.tick as f64 <= scroll_tick)
            .or_else(|| key_sig.first());

        let Some(ev) = eff else {
            // 无调号：白键行浅色，与键盘列黑白键呼应。
            let white = mix(self.theme.app_bg, self.theme.text_primary, 0.06);
            for key in key_lo..=key_hi {
                if !matches!(key % 12, 1 | 3 | 6 | 8 | 10) {
                    band(painter, key, white);
                }
            }
            return;
        };
        let mask = ev.scale.pitch_classes(ev.root);
        let in_scale = mix(self.theme.app_bg, self.theme.text_primary, 0.06);
        let root = mix(self.theme.app_bg, self.theme.text_primary, 0.12);
        for key in key_lo..=key_hi {
            let pc = (key as u8) % 12;
            let color = if pc == ev.root {
                root
            } else if mask & (1u16 << pc) != 0 {
                in_scale
            } else {
                continue;
            };
            band(painter, key, color);
        }
    }

    /// 八度分隔线：每个 C（key % 12 == 0）的顶部一条横线，与桌面端
    /// bg::paint_octave_lines 一致，纵向定位八度。
    fn draw_octave_lines(&self, ui: &egui::Ui, rect: egui::Rect) {
        let kh = self.view.key_height;
        let bottom = 128.0 * kh - self.view.base.scroll_y;
        let painter = ui.painter();
        for key in (0u8..128).step_by(12) {
            let y = rect.min.y + bottom - key as f32 * kh;
            if y < rect.min.y || y > rect.max.y {
                continue;
            }
            painter.line_segment(
                [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
                egui::Stroke::new(1.0, self.theme.line_fg),
            );
        }
    }

    /// 顶部小节标尺：深色底（铺满全宽，含键盘列上方）+ 每小节刻度线 + 小节号。
    /// 与网格线同级别：刻度 = 每小节（变拍子感知），数字只在合并边界显示
    /// （缩放小时每 2/4/8… 小节一个数字），过密时整条隐藏。
    fn draw_ruler(&self, ui: &egui::Ui, rect: egui::Rect, content_rect: egui::Rect, ruler_h: f32) {
        if ruler_h <= 0.0 {
            return;
        }
        let Some(model) = &self.model else {
            return;
        };
        let ppu = self.view.base.pixels_per_tick;
        if ppu <= 0.001 {
            return;
        }
        let scroll_x = self.view.base.scroll_x;
        let tpb = model.meta.ppq.max(1);
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let def_den = def_den.trailing_zeros() as u8;
        let segments = build_time_sig_segments(&model.conductor.time_sig, def_num, def_den);
        let tick_start = scroll_x as f64 / ppu as f64;
        let tick_end = (scroll_x + content_rect.width()) as f64 / ppu as f64;
        const MIN_SPACING: f32 = 38.0;

        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, rect.min.y),
            egui::pos2(rect.max.x, rect.min.y + ruler_h),
        );
        let painter = ui.painter();
        painter.rect_filled(ruler_rect, 0.0, self.theme.track_bg);
        painter.line_segment(
            [
                egui::pos2(ruler_rect.min.x, ruler_rect.max.y),
                egui::pos2(ruler_rect.max.x, ruler_rect.max.y),
            ],
            egui::Stroke::new(1.0, self.theme.line_fg),
        );

        let mut bar_offset = 0u32;
        for (i, &(seg_start, num, den)) in segments.iter().enumerate() {
            let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
            if seg_start as f64 > tick_end {
                break;
            }
            let ticks_per_measure = measure_ticks(tpb, num, den).max(1);
            let measure_px = ticks_per_measure as f32 * ppu;
            if measure_px < 6.0 {
                // 太密整条隐藏（与桌面阈值一致）；段内小节数仍累计。
                let seg_ticks = seg_end.saturating_sub(seg_start) as f32;
                bar_offset += (seg_ticks / ticks_per_measure as f32).ceil() as u32;
                continue;
            }
            let divisor = compute_measure_divisor(measure_px, MIN_SPACING);
            let show_number = measure_px >= 30.0;
            let first_tick = seg_start.max(tick_start as u32);
            let mut tick = seg_start.saturating_add(
                first_tick.saturating_sub(seg_start) / ticks_per_measure * ticks_per_measure,
            );
            while (tick as f64) <= tick_end && tick < seg_end {
                let local = tick - seg_start;
                let x = content_rect.min.x + tick as f32 * ppu - scroll_x;
                if x >= content_rect.min.x && x <= content_rect.max.x {
                    painter.line_segment(
                        [
                            egui::pos2(x, ruler_rect.min.y),
                            egui::pos2(x, ruler_rect.max.y),
                        ],
                        egui::Stroke::new(1.0, self.theme.tick_label),
                    );
                    if show_number && local % (ticks_per_measure * divisor) == 0 {
                        painter.text(
                            egui::pos2(x + 3.0, ruler_rect.min.y + 1.0),
                            egui::Align2::LEFT_TOP,
                            // 小节号从 1 开始（tick 0 = 第 1 小节），跨段累计。
                            (bar_offset + local / ticks_per_measure + 1).to_string(),
                            egui::FontId::proportional(10.0),
                            self.theme.measure_label,
                        );
                    }
                }
                tick += ticks_per_measure;
            }
            let seg_ticks = seg_end.saturating_sub(seg_start) as f32;
            bar_offset += (seg_ticks / ticks_per_measure as f32).ceil() as u32;
        }
    }

    /// 播放光标：竖线随 scroll_x 移动，y 从标尺底部到内容区底。
    /// 光标不在视口内时不画——贴边显示会误导"线就在那里"。
    fn draw_cursor(&self, ui: &egui::Ui, rect: egui::Rect, ruler_h: f32) {
        let Some(tick) = self.cursor_tick else {
            return;
        };
        let ppu = self.view.base.pixels_per_tick;
        let scroll_x = self.view.base.scroll_x;
        let left = rect.min.x + self.keyboard_w;
        let x = left + (tick as f32 * ppu - scroll_x);
        if x < left || x > rect.max.x {
            return;
        }
        let painter = ui.painter();
        painter.line_segment(
            [
                egui::pos2(x, rect.min.y + ruler_h),
                egui::pos2(x, rect.max.y),
            ],
            egui::Stroke::new(2.0, self.theme.accent_active),
        );
    }

    /// 网格线分层（与桌面端 grid_lines 一致，支持变拍子）：
    /// 小节线 2px、四分音符线 1px、十六分线 1px、tick 线（最大缩放）。
    /// 每拍号段从段起点 local 对齐，变拍子段不会错位。
    fn draw_grid(&self, ui: &egui::Ui, rect: egui::Rect) {
        let Some(model) = &self.model else {
            return;
        };
        let ppu = self.view.base.pixels_per_tick;
        if ppu <= 0.001 {
            return;
        }
        let scroll_x = self.view.base.scroll_x;
        let left = rect.min.x;
        let right = rect.max.x;
        let tick_start = scroll_x as f64 / ppu as f64;
        let tick_end = (scroll_x + rect.width()) as f64 / ppu as f64;
        let tpb = model.meta.ppq.max(1);
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let def_den = def_den.trailing_zeros() as u8;
        let segments = build_time_sig_segments(&model.conductor.time_sig, def_num, def_den);
        let painter = ui.painter();
        // 网格线 = 标尺标签的下一级（与桌面 grid_lines 同规则）：
        // 合并小节标签 → 合并/2 小节线；每小节标签 → 四分音符线；
        // beat 标签 → 十六分线；sub 标签 → tick 线（仅最大缩放）。
        const MIN_SPACING: f32 = 38.0;
        const SUB_BEAT_DIV: u32 = 4;
        // tick 线只在最大缩放显示：安卓 ppu 上限 8.0（与 handle_touch clamp 一致），
        // 桌面端 10.0 在这里永远不可达，tick 线会永不显示。
        const MAX_PPU: f32 = 8.0;
        let ticks_per_sub = (tpb / SUB_BEAT_DIV).max(1);

        for (i, &(seg_start, num, den)) in segments.iter().enumerate() {
            let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
            let seg_start_f = seg_start as f64;
            if seg_start_f > tick_end {
                break;
            }

            let ticks_per_measure = measure_ticks(tpb, num, den).max(1);
            let ticks_per_beat = (ticks_per_measure / num.max(1) as u32).max(1);
            let measure_divisor =
                compute_measure_divisor(ticks_per_measure as f32 * ppu, MIN_SPACING);
            let merged_measure_ticks = ticks_per_measure.saturating_mul(measure_divisor);
            let merged = measure_divisor > 1;
            let show_beat = !merged;
            let show_sub = show_beat && (ticks_per_beat as f32 * ppu) >= MIN_SPACING;
            let show_tick = show_sub && ppu >= MAX_PPU;
            let grid_measure_ticks = if merged {
                (merged_measure_ticks / 2).max(1)
            } else {
                ticks_per_measure
            };
            let step = if show_tick {
                1u32
            } else if show_sub {
                ticks_per_sub
            } else if show_beat {
                ticks_per_beat
            } else {
                grid_measure_ticks.max(1)
            };

            // 段内 local 对齐：变拍子段的 seg_start 通常不在全局 step 网格上。
            let first_tick = seg_start_f.max(tick_start);
            let step_f = step as f64;
            let first = seg_start.saturating_add(
                (((first_tick - seg_start_f) / step_f).floor() as u32).saturating_mul(step),
            );

            let mut tick = first;
            while (tick as f64) <= tick_end && tick < seg_end {
                let local = tick - seg_start;
                let x = rect.min.x + tick as f32 * ppu - scroll_x;
                if x >= left && x <= right {
                    let is_measure = local % ticks_per_measure == 0;
                    let beat_local = local % ticks_per_measure;
                    let is_beat_pos = beat_local.is_multiple_of(ticks_per_beat) && beat_local > 0;
                    let is_sub_pos = local % ticks_per_sub == 0;
                    let (w, color) = if is_measure {
                        (2.0, self.theme.line_fg)
                    } else if show_beat && is_beat_pos {
                        (1.0, self.theme.text_label)
                    } else if show_sub && is_sub_pos {
                        (1.0, self.theme.grid_sub_beat)
                    } else if show_tick {
                        (1.0, self.theme.grid_tick)
                    } else {
                        tick += step;
                        continue;
                    };
                    painter.line_segment(
                        [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                        egui::Stroke::new(w, color),
                    );
                }
                tick += step;
            }
        }
    }
}

/// 创建离屏渲染纹理并注册为 egui 纹理（与桌面 RenderContext 同款）。
fn create_target(
    device: &wgpu::Device,
    egui_renderer: &mut eframe::egui_wgpu::Renderer,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, egui::TextureId) {
    let max_dim = device.limits().max_texture_dimension_2d;
    let width = width.min(max_dim).max(1);
    let height = height.min(max_dim).max(1);
    let linear_format = match format {
        wgpu::TextureFormat::Bgra8UnormSrgb => Some(wgpu::TextureFormat::Bgra8Unorm),
        wgpu::TextureFormat::Rgba8UnormSrgb => Some(wgpu::TextureFormat::Rgba8Unorm),
        _ => None,
    };
    let view_formats: &[wgpu::TextureFormat] = if let Some(lf) = &linear_format {
        std::slice::from_ref(lf)
    } else {
        &[]
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pianoroll_preview"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats,
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let texture_id =
        egui_renderer.register_native_texture(device, &view, wgpu::FilterMode::Nearest);
    (texture, view, texture_id)
}
