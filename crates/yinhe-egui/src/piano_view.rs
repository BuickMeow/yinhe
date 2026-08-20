use eframe::egui;
use rust_i18n::t;

use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;
use yinhe_types::{AutomationLane, TimeSigEvent};

use crate::widgets::selection_actions::SelectionAction;
use crate::widgets::tools_panel::Tool;
pub use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
pub use yinhe_types::PencilNoteDrag;

pub mod automation_panel;
mod bg;
pub mod control_bar;
pub(crate) mod drag;
pub(crate) mod gpu_upload;
mod keyboard;
mod marquee;
mod pencil;
mod perf;

/// Events emitted by the piano-roll view for the caller to act on.
pub enum PianoViewEvent {
    SelectionAction(SelectionAction),
    AddNote {
        track: u16,
        note: yinhe_core::NoteEvent,
    },
    EraserDelete {
        t_start: u32,
        t_end: u32,
        key_lo: u8,
        key_hi: u8,
        track_lo: u16,
        track_hi: u16,
    },
}

/// Automation panel 上下文（all-or-nothing：要么全 Some 要么全 None）。
/// 合并 5 个 auto_* 参数，减少 piano_view::show 的参数数量。
pub struct AutomationPanelsCtx<'a> {
    pub panels: &'a mut Vec<yinhe_types::AutomationPanelView>,
    pub renderers: &'a mut Vec<(
        yinhe_wgpu::InstanceRenderer,
        crate::render_context::RenderContext,
    )>,
    pub lanes: &'a [yinhe_types::AutomationLane],
    /// 渲染用 lanes：所有 PR 可见音轨的 lanes（与音符显示逻辑一致）。
    /// `lanes` 仅为主音轨的编辑目标，渲染不受其限制。
    pub render_lanes: &'a [&'a yinhe_types::AutomationLane],
    pub show: &'a mut bool,
    pub wgpu_state: &'a std::sync::Arc<eframe::egui_wgpu::RenderState>,
}

/// 音符听觉预览请求（UI 交互 → App → AudioCommand）。
/// 预览音从目标音轨的通道发出，通道状态按目标位置（target_tick）的自动化。
pub(crate) enum PreviewReq {
    /// 播放/重触发一个音符预览。`duration_ticks == 0` 表示持续音（直到 `Stop`）。
    Note(NotePreview),
    /// 停止持续音预览。
    Stop,
}

/// 单个音符的预览参数。
pub(crate) struct NotePreview {
    pub track: u16,
    pub key: u8,
    /// `None` = 用该音轨最近修改力度（default_velocity）。
    pub velocity: Option<u8>,
    /// 目标位置 tick：自动化状态采样点（音符起点）。
    pub target_tick: u32,
    /// 预览时长（tick），0 = 持续音（配合 `PreviewReq::Stop`）。
    pub duration_ticks: u32,
}

/// piano_view 给外部的反馈通道（合并多个 &mut 出参）。
pub struct PianoViewFeedback<'a> {
    pub auto_edit_events: &'a mut Vec<crate::piano_view::automation_panel::AutomationEdit>,
    pub info_content: &'a mut Option<crate::right_panel::InfoContent>,
    pub right_tab: &'a mut Option<crate::right_panel::RightTab>,
    pub automation_drag_ghost: &'a mut Option<(u32, f32)>,
    pub note_drag_delta: &'a mut Option<(i64, i32, bool)>,
    pub pencil_note_drag: &'a mut Option<PencilNoteDrag>,
    /// 选框边缘拖动伸缩：(side, delta_ticks)。dt 按量化对齐。
    pub note_resize_delta: &'a mut Option<(ResizeSide, i64)>,
    pub velocity_edits: &'a mut Vec<yinhe_types::VelocityEdit>,
    /// 音符听觉预览请求（铅笔新建/拖拽、选框拖拽触发）。
    pub preview_reqs: &'a mut Vec<PreviewReq>,
    /// 状态栏讲解行：钢琴卷帘悬停提示（位置 + 音高）。
    pub status_hint: &'a mut Option<String>,
    /// 控制栏事件（量化/切换主音轨/显示音轨勾选），由 layout 应用。
    pub bar_events: &'a mut Vec<control_bar::PrBarEvent>,
}

/// Height of the time ruler band at the top of the pianoroll view.
use crate::theme;
const RULER_H: f32 = theme::RULER_H;

/// 按住 Alt（Option）时的有效工具（Cubase 风格临时切换）：
/// - Select/SelectVertical + Alt：悬停在音符或选框上 = 保持选择（Alt 拖拽复制）；
///   悬停空白 = 临时铅笔（画音符）。
///   例外：选框拖拽状态机进行中（含 Alt 克隆）时锁定选择工具——
///   拖拽中鼠标移出音符原位后 hover 命中会失败，不得据此切成铅笔。
/// - Pencil + Alt = 临时选择（框选/移动）。
/// - 其余工具不受影响。
///
/// 自动化面板不使用该映射（Alt 在那里是"复制锚点"）。
#[allow(clippy::too_many_arguments)]
fn effective_tool(
    ui: &egui::Ui,
    active: Tool,
    midi: Option<&dyn yinhe_types::NoteSource>,
    view: &yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
    sel_rect: &yinhe_editor_core::edit_state::SelRectState,
) -> Tool {
    if !ui.input(|i| i.modifiers.alt) {
        return active;
    }
    match active {
        Tool::Pencil => Tool::Select,
        Tool::Select | Tool::SelectVertical => {
            // 拖拽进行中（含 Alt 克隆）→ 锁定选择工具，不得切成铅笔。
            if drag::sel_drag_in_progress(ui) {
                return active;
            }
            // 悬停音符或选框 = 保留选择（Alt 拖拽复制）；空白 = 临时铅笔。
            let hit = ui.input(|i| i.pointer.hover_pos()).is_some_and(|pos| {
                if !music_rect.contains(pos) {
                    return false;
                }
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                drag::hit_test_note(midi, view, local, track_visible, track_selected).is_some()
                    || sel_rect.effective_rects().iter().any(|&(t0, t1, k0, k1)| {
                        crate::selection::drag::music_sel_to_pixel_rect(view, t0, t1, k0, k1)
                            .contains(local)
                    })
            });
            if hit { active } else { Tool::Pencil }
        }
        t => t,
    }
}

/// Display the pianoroll texture with zoom/pan interaction.
///
/// When `auto_*` parameters are `Some`, automation panels are rendered between
/// the pianoroll content and the horizontal scrollbar. The AUTO toggle and
/// +/- buttons live inside the scrollbar's left blank area (same width as the
/// piano keyboard).
///
/// Returns an optional event for the caller to handle (selection action or
/// note-add request).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    pianoroll: &mut yinhe_wgpu::InstanceRenderer,
    render_ctx: &mut super::render_context::RenderContext,
    render_thread: Option<&yinhe_wgpu::RenderThreadHandle>,
    view: &mut yinhe_types::PianoRollView,
    last_cull_revision: &mut u64, // revision ^ hidden_hash — triggers all_notes re-upload
    last_cull_revision_only: &mut u64, // last revision for incremental detection
    last_hidden_hash: &mut u64,   // last hidden_hash for incremental detection
    last_tv_hash: &mut u64,       // last track_visible hash (track_mask 变化检测)
    last_hidden_keys: &mut crate::piano_view::gpu_upload::HiddenKeyMask, // last hidden_notes key 位图（hidden 增量重建判定）
    cull_rebuild: &mut Option<crate::piano_view::gpu_upload::CullRebuild>, // 后台重建状态机
    midi: Option<&dyn yinhe_types::NoteSource>,
    midi_arc: Option<&std::sync::Arc<yinhe_core::YinModel>>,
    selected: &mut yinhe_core::Selection,
    track_visible: &[bool],
    track_colors: &[[f32; 4]],
    cursor_tick: &mut Option<f64>,
    is_playing: bool,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    key_sig_events: &[yinhe_types::KeySigEvent],
    last_cursor_tick: &mut Option<f64>,
    follow_mode: &mut super::view_interaction::FollowMode,
    active_tool: &Tool,
    // Automation panel data (all-or-nothing)
    mut auto_ctx: Option<AutomationPanelsCtx<'_>>,
    scroll_mode: u32,
    min_border_width: f32,
    content_opacity: f32,
    note_outline: bool,
    use_gpu_cull: bool,
    tempo_lane: &AutomationLane,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    track_selected: &std::collections::HashSet<u16>,
    conductor_idx: Option<u16>,
    // 写入目标轨 = 主音轨或回退轨（无选中时第一个非 Conductor 轨），由 layout 计算。
    write_track: Option<u16>,
    // 控制栏输入（标尺下方一栏：量化/音轨名称/和弦指示器）。
    bar: control_bar::PrBarData<'_>,
    revision: u64,
    note_revisions: &[u64; yinhe_types::KEY_COUNT],
    feedback: &mut PianoViewFeedback<'_>,
    sel_hint: Option<&crate::app::layout::SelHintInfo>,
) -> Option<PianoViewEvent> {
    // Sense::hover() — no drag ownership. All drag is handled by dedicated
    // ui.interact calls below, each inside its own push_id scope.
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::hover());
    let rect = resp.rect;

    // Compute automation panel natural total height.
    // First panel has no leading handle; subsequent panels have SPLIT_H above them.
    let panels_natural_h: f32 = match &auto_ctx {
        Some(ctx) if *ctx.show && !ctx.panels.is_empty() => {
            ctx.panels.iter().map(|p| p.panel_height).sum::<f32>()
                + (ctx.panels.len() as f32 * automation_panel::SPLIT_H)
        }
        _ => 0.0,
    };

    // ── 布局：方向感知（主轴 = 时间轴）──
    // 横向：竖 ruler 顶部横条 + 左侧键盘列；底部水平 scrollbar（时间），右侧竖 scrollbar（音高）。
    // 纵向瀑布流：control bar 顶部、竖 ruler 左侧列、键盘底部横条（高 = keyboard_width）；
    //   右侧竖 scrollbar（时间），底部横 scrollbar（音高）。
    let vertical = view.is_vertical();
    let kb_w = view.keyboard_width();
    let content_right_x = rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W;

    // AM 面板可用高度（横向：content 之下；纵向：content 之下、键盘之上）
    let avail_h = if vertical {
        (rect.height() - theme::PR_BAR_H - crate::widgets::scrollbar::SCROLLBAR_H - kb_w).max(0.0)
    } else {
        (rect.height() - RULER_H - theme::PR_BAR_H - crate::widgets::scrollbar::SCROLLBAR_H)
            .max(0.0)
    };
    let panels_max_h = (avail_h * 0.65).max(0.0);
    let panels_total_h = panels_natural_h.min(panels_max_h);

    // 音乐区位置：横向顶部从 ruler+control_bar 之下开始；纵向顶部从 control_bar 之下。
    let ruler_band_y = rect.min.y;
    let (content_y, content_bottom, content_left_x, music_left_x) = if vertical {
        let top = rect.min.y + theme::PR_BAR_H;
        // 底部：key 滚动条 + 键盘条（高 kb_w）
        let keyboard_top = rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H - kb_w;
        let bottom = keyboard_top - panels_total_h;
        let left = rect.min.x + RULER_H; // 竖 ruler 列
        (top, bottom.max(top), left, left)
    } else {
        let top = rect.min.y + RULER_H + theme::PR_BAR_H;
        let bottom = top + (avail_h - panels_total_h).max(0.0);
        (top, bottom, rect.min.x, rect.min.x + kb_w)
    };
    let content_h = (content_bottom - content_y).max(0.0);
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(content_left_x, content_y),
        egui::pos2(content_right_x, content_bottom),
    );
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(music_left_x, content_y),
        egui::pos2(content_right_x, content_bottom),
    );
    let ppp = ui.ctx().pixels_per_point();
    let w = content_rect.width() as u32;
    let h = content_rect.height() as u32;
    let pw = (w as f32 * ppp) as u32;
    let ph = (h as f32 * ppp) as u32;

    if w == 0 || h == 0 {
        return None;
    }

    // ── Perf probe (only when YIN_PERF=1) ──
    let perf_on = yinhe_memtrace::perf_probe::enabled();
    let t_show_start = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Resize render target if needed — texture_id may change after this
    render_ctx.ensure_size(pw, ph);

    // Update render thread's target view if texture was recreated
    if let Some(rt) = render_thread {
        // 目标视口用纹理实际尺寸（超 GPU 上限时被裁剪，视口必须一致）
        let (aw, ah) = render_ctx.actual_size();
        rt.update_target(render_ctx.preview_view().clone(), aw, ah);
    }

    // Clamp scroll — add some extra space beyond the last note
    let total_ticks = super::view_interaction::total_ticks_padded(
        midi.and_then(|m| m.tick_length()).unwrap_or(0),
        ppq,
    );
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // Auto-follow: scroll based on follow mode (playback only).
    // Never auto-follow when paused, so the user can freely scroll around.
    // 触发（居中/翻页/连续）后向目标 scroll_x 帧间指数插值：跳变变成平滑
    // 滑动，代替逐帧硬设置（原实现看起来像高速翻页）。非播放时清空插值
    // 目标，避免恢复播放后画面自行滚向旧目标。
    let follow_active = is_playing && *follow_mode != super::view_interaction::FollowMode::None;
    if !follow_active {
        view.base.follow_target = None;
    } else if let Some(ct) = *cursor_tick {
        let dt = ui.input(|i| i.stable_dt).max(1e-4);
        // 沿主轴跟随：横向滚动目标 = scroll_x（视口宽 w）；纵向 = scroll_y（视口高 h，
        // 时间轴起点在顶部、无键盘列偏移）。compute_follow_scroll 数学单轴通用。
        let (main_len, left_boundary, cur_main) = if view.is_vertical() {
            (h as f32, 0.0, view.base.scroll_y)
        } else {
            (w as f32, view.keyboard_width(), view.base.scroll_x)
        };
        if let Some(t) = super::view_interaction::compute_follow_scroll(
            ct,
            view.base.pixels_per_tick,
            main_len,
            left_boundary,
            *follow_mode,
            1.0,
            cur_main,
        ) {
            view.base.follow_target = Some(t);
        }
        if let Some(t) = view.base.follow_target {
            let before = *view.main_scroll();
            *view.main_scroll() = super::view_interaction::follow_interpolate(
                before,
                t,
                dt,
                super::view_interaction::FOLLOW_TAU,
            );
            view.clamp_scroll(w as f32, h as f32, total_ticks);
            // 已到达目标（1px 数值容差）或滚动被 clamp 卡在边界：结束插值。
            if (t - *view.main_scroll()).abs() <= 1.0 || *view.main_scroll() == before {
                view.base.follow_target = None;
            }
        }
    }

    // ── Selection drag (Select tool only) ──
    // Update state BEFORE handle_input to avoid egui pointer-capture conflicts.
    let mut sel_action = None;
    let mut pencil_event: Option<PianoViewEvent> = None;
    let mut eraser_event: Option<PianoViewEvent> = None;
    let mut ghost_notes: Vec<(u32, u32, u8, u16)> = Vec::new();
    let mut hidden_notes: std::collections::HashSet<(u16, u32, u8)> =
        std::collections::HashSet::new();
    // 按住 Alt 时 Select↔Pencil 双向临时切换（详见 effective_tool）。
    let effective_tool = effective_tool(
        ui,
        *active_tool,
        midi,
        view,
        content_rect,
        music_rect,
        track_visible,
        track_selected,
        sel_rect,
    );
    if effective_tool == Tool::Select || effective_tool == Tool::SelectVertical {
        let vertical = effective_tool == Tool::SelectVertical;
        let (sel_ghosts, sel_hidden, sel_previews, sel_note_event, sel_pencil_drag) =
            drag::sel_drag_frame(
                ui,
                content_rect,
                music_rect,
                view,
                midi,
                selected,
                quantize,
                ppq,
                bar_line_data,
                total_ticks,
                cursor_tick,
                feedback.note_drag_delta,
                feedback.note_resize_delta,
                sel_rect,
                track_colors,
                track_visible,
                track_selected,
                write_track,
                conductor_idx,
                vertical,
            );
        ghost_notes = sel_ghosts;
        hidden_notes = sel_hidden.into_iter().collect();
        feedback.preview_reqs.extend(sel_previews);
        // 双击写音符（选择工具）：与铅笔一致，目标轨 = write_track。
        if let Some((note, track)) = sel_note_event {
            pencil_event = Some(PianoViewEvent::AddNote { track, note });
        }
        // 单音符边缘伸缩（选择工具，不用先选中）：复用铅笔的提交通道。
        *feedback.pencil_note_drag = sel_pencil_drag;
    } else if effective_tool == Tool::Pencil {
        let (note_event, ghost, hidden, pencil_drag, preview) = pencil::pencil_frame(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            write_track,
            track_visible,
            conductor_idx,
            midi,
            track_colors,
            total_ticks,
        );
        ghost_notes = ghost;
        hidden_notes.extend(hidden);
        *feedback.pencil_note_drag = pencil_drag;
        if let Some(p) = preview {
            feedback.preview_reqs.push(p);
        }
        if let Some(note) = note_event
            && let Some(track) =
                pencil::valid_pencil_track(write_track, track_visible, conductor_idx)
        {
            pencil_event = Some(PianoViewEvent::AddNote { track, note });
        }
    } else if effective_tool == Tool::Eraser {
        eraser_event = marquee::eraser_drag_frame(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            track_selected,
        );
    }

    // ── Hover cursor: show Move/ResizeWest/ResizeEast when over selection rect ──
    if (effective_tool == Tool::Select || effective_tool == Tool::SelectVertical)
        && !crate::view_interaction::pointer_over_popup(ui.ctx())
        && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && music_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let eff_rects = sel_rect.effective_rects();
        // 音符 hit-test 优先（不用先选中，与铅笔一致）：
        // 边缘 → 伸缩光标；中部 → 移动光标。
        if let Some((mode, _, _, _, _)) =
            drag::hit_test_note(midi, view, local, track_visible, track_selected)
        {
            use crate::piano_view::pencil::HitMode;
            match mode {
                HitMode::ResizeLeft => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest),
                HitMode::ResizeRight => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast),
                HitMode::Move => ui.ctx().set_cursor_icon(egui::CursorIcon::Move),
            }
        } else if let Some((side, _, _)) = drag::hit_test_sel_edge(&eff_rects, view, local) {
            match side {
                yinhe_editor_core::ResizeSide::Left => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest)
                }
                yinhe_editor_core::ResizeSide::Right => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast)
                }
            }
        } else {
            let in_sel_rect = eff_rects.iter().any(|&(t_start, t_end, key_lo, key_hi)| {
                let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
                    view, t_start, t_end, key_lo, key_hi,
                );
                pixel_rect.contains(local)
            });
            if in_sel_rect {
                // 垂直选框（垂直工具或空区域框选自动生成的全键选框）：只能水平拖动
                // → 左右双向指针；普通选框工具：四向移动指针。
                let icon = if effective_tool == Tool::SelectVertical || sel_rect.has_auto_vertical()
                {
                    egui::CursorIcon::ResizeHorizontal
                } else {
                    egui::CursorIcon::Move
                };
                ui.ctx().set_cursor_icon(icon);
            }
        }
    }

    // ── Content interaction (zoom/pan/cursor/drag/reset) ──
    // 横向：左区（键盘列）垂直缩放、右区平移/水平缩放，x_to_tick 内部减 left_panel_width。
    // 纵向：无左区（键盘在底部），滚轮/缩放直接按主轴（时间轴沿 Y）分发。
    let left_zone = if view.is_vertical() { 0.0 } else { kb_w };
    crate::view_interaction::handle_input(
        ui,
        content_rect,
        view,
        cursor_tick,
        left_zone,
        Some((quantize, ppq)),
        bar_line_data,
        None,
        None, // 命中区域 = content_rect（PR 无左列外区域）
        is_playing,
        follow_mode,
        active_tool,
    );

    // ── Keyboard resize handle ──
    // 横向：左侧键盘列的右缘竖线；纵向：底部键盘条的顶部横线。
    ui.push_id("kb_handle", |ui| {
        let vertical = view.is_vertical();
        let handle_rect = if vertical {
            let hy = rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H - view.keyboard_width();
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, hy - 2.0),
                egui::pos2(content_right_x, hy + 2.0),
            )
        } else {
            let handle_x = rect.min.x + view.keyboard_width();
            egui::Rect::from_min_max(
                egui::pos2(handle_x - 2.0, rect.min.y),
                egui::pos2(handle_x + 2.0, content_rect.max.y),
            )
        };
        let handle_resp = ui.interact(handle_rect, ui.id(), egui::Sense::click_and_drag());
        // 只有按下位置真的在把手矩形内才响应拖动：egui 的 interact_radius
        // 会把把手附近 ~5px 的按下判为命中，导致拖动自动化锚点/分割线时
        // 误拖键盘宽度（一次只能按一个物品）。
        let on_handle = ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| handle_rect.contains(p));
        let press_on_handle = ui
            .input(|i| i.pointer.press_origin())
            .is_some_and(|p| handle_rect.contains(p));
        if on_handle && (handle_resp.hovered() || handle_resp.dragged()) {
            ui.ctx().set_cursor_icon(if vertical {
                egui::CursorIcon::ResizeVertical
            } else {
                egui::CursorIcon::ResizeHorizontal
            });
        }
        if press_on_handle && handle_resp.dragged() {
            let delta = if vertical {
                handle_resp.drag_delta().y
            } else {
                handle_resp.drag_delta().x
            };
            let old_kb = view.keyboard_width();
            let new_kb = (old_kb + delta).clamp(
                crate::theme::MIN_KEYBOARD_WIDTH,
                rect.width() * crate::theme::MAX_KEYBOARD_RATIO,
            );

            // 主视口长度随键盘条尺寸变化，按比例换算保持 start_tick 不变：
            // 横向 = 音乐区宽 w - kb；纵向 = 音乐区高 h - kb（主轴长度变化）。
            let old_main = (if vertical { h as f32 } else { w as f32 }) - old_kb;
            let new_main = (if vertical { h as f32 } else { w as f32 }) - new_kb;
            if old_main > 0.0 && new_main > 0.0 {
                let start_tick = view.main_scroll_val() / view.base.pixels_per_tick;
                let new_start_tick = start_tick * old_main / new_main;
                *view.main_scroll() = new_start_tick * view.base.pixels_per_tick;
            }

            view.base.left_panel_width = new_kb;
            view.base.dirty = true;
            ui.ctx().request_repaint();
        }
    });

    // ── Clamp scroll after all interactions ──
    let total_ticks = super::view_interaction::total_ticks_padded(
        midi.map(|m| m.tick_length().unwrap_or(0)).unwrap_or(0),
        ppq,
    );
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // ── Dirty detection ──
    // cursor_tick no longer affects rendering at all — the cursor is drawn
    // by egui directly on top of the wgpu texture, outside the cache.
    // app_eframe already calls request_repaint while audio is playing.
    *last_cursor_tick = *cursor_tick;

    // Perf probe: capture input phase duration (everything up to prepare).
    let t_input_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // ── Upload all notes to GPU cull buffer ──
    // Only when GPU cull mode is enabled. CPU build mode skips this entirely.
    // 增量 per-key 上传 → 失败回退全量上传。详见 gpu_upload 模块。
    if use_gpu_cull {
        gpu_upload::upload(gpu_upload::GpuUploadState {
            pianoroll,
            midi,
            midi_arc,
            revision,
            note_revisions,
            track_visible,
            hidden_notes: &hidden_notes,
            last_cull_revision,
            last_cull_revision_only,
            last_hidden_hash,
            last_tv_hash,
            last_hidden_keys,
            rebuild: cull_rebuild,
        });
    }

    // Prepare GPU data (ghost notes are handled separately as a transient overlay)
    let theme = pianoroll.theme().clone();
    let cull_ready = use_gpu_cull && pianoroll.cull_ready();
    tracing::debug!(
        "[cull-frame] cull_ready={cull_ready} scroll_x={} scroll_y={} ppu={} kh={} w={w} h={h}",
        view.base.scroll_x,
        view.base.scroll_y,
        view.base.pixels_per_tick,
        view.key_height,
    );
    if cull_ready {
        // GPU cull path: upload ghost layer (GPU cull handles notes)
        let job = yinhe_wgpu::build_render_job(
            w,
            h,
            view,
            &*selected,
            track_colors,
            scroll_mode,
            min_border_width,
            note_outline,
        );
        pianoroll.upload_uniforms(job.uniforms);
        pianoroll.upload_track_colors(&job.track_colors);
        pianoroll.upload_selection(&job.selection);
        // Grid 已迁移到 egui，wgpu 只剩 ghost note overlay 一层。
        pianoroll.ensure_layers(1);
        // Ghost note overlay: built and uploaded independently.
        // Always upload (even when empty) to clear stale ghost data from the previous frame.
        pianoroll.upload_note_layer(0, 0, |out| {
            for &(start_tick, end_tick, key, track) in &ghost_notes {
                yinhe_wgpu::build_ghost_note(out, start_tick, end_tick, key, track, &theme);
            }
        });
    } else if let Some(rt) = render_thread {
        // Async path (no cull): build instances on this thread, send to render thread
        let job = yinhe_wgpu::build_render_job(
            w,
            h,
            view,
            &*selected,
            track_colors,
            scroll_mode,
            min_border_width,
            note_outline,
        );
        // Build note instances + ghost overlay as note layers for the render thread.
        let mut notes_instances = Vec::new();
        if let Some(midi) = midi {
            yinhe_wgpu::build_notes(
                &mut notes_instances,
                w as f32,
                h as f32,
                midi,
                view,
                &hidden_notes,
                track_visible,
            );
        }
        let mut ghost_instances = Vec::new();
        for &(start_tick, end_tick, key, track) in &ghost_notes {
            yinhe_wgpu::build_ghost_note(
                &mut ghost_instances,
                start_tick,
                end_tick,
                key,
                track,
                &theme,
            );
        }
        // Cache key for notes: includes viewport tick/key range, revision,
        // hidden_notes, and track_visible — anything that affects which notes
        // are built. When none of these change (e.g. steady state, no scroll/
        // edit), the render thread skips GPU upload entirely.
        let (tick_start, tick_end) =
            view.visible_main_range(view.main_axis_len(w as f32, h as f32));
        let (key_lo, key_hi) = view.visible_cross_range(view.cross_axis_len(w as f32, h as f32));
        let tv_hash = yinhe_wgpu::hash_bools(track_visible);
        let hidden_hash = yinhe_wgpu::hash_hidden(&hidden_notes);
        let notes_cache_key = yinhe_wgpu::layer_cache_key(&[
            tick_start.to_bits(),
            tick_end.to_bits(),
            key_lo as u64,
            key_hi as u64,
            tv_hash,
            revision,
            hidden_hash,
        ]);
        let note_layers = vec![
            yinhe_wgpu::NoteLayerData {
                instances: notes_instances,
                cache_key: notes_cache_key,
                force: false,
            },
            yinhe_wgpu::NoteLayerData {
                instances: ghost_instances,
                cache_key: 0,
                force: true,
            },
        ];
        rt.send_job(yinhe_wgpu::RenderJob {
            width: job.width,
            height: job.height,
            uniforms: job.uniforms,
            track_colors: job.track_colors,
            selection: job.selection,
            note_layers,
        });
    }

    let t_prepare_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Static cache was removed — every frame rebuilds + uploads, so always paint.
    view.base.dirty = false;

    // ── Background（app_bg 一层，不透明不叠加；条纹/色块自行叠上）──
    let theme = pianoroll.theme().clone();
    painter.rect_filled(content_rect, 0.0, crate::theme::app_bg());

    // ── Scale background + 八度横线（调号驱动的调内/调外/根音条带）──
    let kh = view.key_height;
    let kb_w = view.keyboard_width();
    bg::paint(
        &painter,
        content_rect,
        kb_w,
        kh,
        view,
        key_sig_events,
        content_opacity,
    );

    // ── Grid lines (drawn by egui before wgpu texture) ──
    // 替代原 wgpu grid layer。与 time_ruler 共用 MIN_SPACING 阈值，保证"有线就有标签"。
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        // 横向：从键盘列右缘开始；纵向：整块音乐区（时间轴沿 Y，线横着）。
        let grid_rect = if view.is_vertical() {
            content_rect
        } else {
            egui::Rect::from_min_max(
                egui::pos2(
                    content_rect.min.x + view.keyboard_width(),
                    content_rect.min.y,
                ),
                content_rect.max,
            )
        };
        crate::widgets::grid_lines::paint_grid_lines(
            &painter,
            grid_rect,
            &view.base,
            tpb,
            def_num,
            def_den,
            sig_events,
            &crate::widgets::grid_lines::GridColors::pianoroll(),
            view.orientation(),
        );
    }

    // Paint wgpu content into the content_rect (notes only — grid moved to egui)
    if cull_ready {
        // GPU cull path: draw directly (no render thread needed — cull makes GPU work fast)
        render_ctx.paint(
            pianoroll,
            pw,
            ph,
            "pianoroll_frame",
            &painter,
            content_rect,
            true,
        );
    } else {
        // Render thread handles GPU work — just display the latest texture
        render_ctx.paint_texture_only(pw, ph, &painter, content_rect);
    }

    let t_paint_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // ── Keyboard (drawn by egui on top of the wgpu texture) ──
    // 横向 = 左侧键盘列；纵向 = 底部横键盘条（高 = kb_w）。
    let keyboard_rect = if view.is_vertical() {
        let kb_bottom = rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H;
        egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, kb_bottom - kb_w),
            egui::pos2(content_right_x, kb_bottom),
        )
    } else {
        egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, content_rect.min.y),
            egui::pos2(content_rect.min.x + kb_w, content_rect.max.y),
        )
    };
    keyboard::paint(&painter, keyboard_rect, kb_w, kh, view, &theme);

    // ── Playback cursor (drawn by egui on top of the wgpu texture) ──
    // Decoupled from the wgpu pipeline so cursor movement during playback
    // does NOT invalidate the static instance cache.
    if let Some(ct) = *cursor_tick {
        let kb_w = view.keyboard_width();
        if view.is_vertical() {
            // 纵向瀑布流：时间沿 Y，游标横线。
            let cy_local = view.tick_to_main_px(ct);
            if cy_local >= 0.0 && cy_local <= h as f32 {
                let cy = content_rect.min.y + cy_local;
                painter.line_segment(
                    [
                        egui::pos2(content_rect.min.x, cy),
                        egui::pos2(content_rect.max.x, cy),
                    ],
                    egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::contrast_fg()),
                );
            }
        } else {
            let cx_local = view.tick_to_x(ct);
            if cx_local >= kb_w && cx_local <= w as f32 {
                let cx = content_rect.min.x + cx_local;
                painter.line_segment(
                    [
                        egui::pos2(cx, content_rect.min.y),
                        egui::pos2(cx, content_rect.max.y),
                    ],
                    egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::contrast_fg()),
                );
            }
        }
    }

    // ── Draw selection box on TOP of GPU content ──
    // State was already updated by sel_drag_frame above; this just draws the box
    // after the GPU paint so it's not covered by the texture.
    if effective_tool == Tool::Select || effective_tool == Tool::SelectVertical {
        let vertical = effective_tool == Tool::SelectVertical;

        // Draw active drag box (if any)
        marquee::draw_marquee_box(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            "sel_drag",
            crate::theme::contrast_fg(),
            crate::theme::contrast_fg(),
            vertical,
        );

        // Draw persisted selection rects (remains after mouse release).
        // Compute pixel rect from music coordinates each frame so it follows
        // scroll/zoom. 多选框时遍历所有 rects。
        let eff_rects = sel_rect.effective_rects();
        let persisted_pixel_rects: Vec<egui::Rect> = eff_rects
            .iter()
            .map(|&(t_start, t_end, key_lo, key_hi)| {
                crate::selection::drag::music_sel_to_pixel_rect(
                    view, t_start, t_end, key_lo, key_hi,
                )
            })
            .collect();
        {
            // 横向：像素 rect 相对 content（含键盘列），转成 music-relative 再相对 music_rect 画；
            // 纵向：music_rect == content_rect，无偏移。
            let kb_w = if view.is_vertical() {
                0.0
            } else {
                music_rect.min.x - content_rect.min.x
            };
            let music_rect_local = egui::Rect::from_min_max(
                egui::pos2(0.0, 0.0),
                egui::pos2(music_rect.width(), music_rect.height()),
            );
            for &rect in &persisted_pixel_rects {
                let shifted = egui::Rect::from_min_max(
                    egui::pos2(rect.min.x - kb_w, rect.min.y),
                    egui::pos2(rect.max.x - kb_w, rect.max.y),
                );
                if shifted.intersects(music_rect_local) {
                    crate::selection::draw::draw(
                        ui.painter(),
                        music_rect,
                        shifted,
                        crate::theme::contrast_fg(),
                        crate::theme::contrast_fg(),
                    );
                }
            }
        }

        // Show floating action bar next to the latest persisted selection rect
        if let Some(action) = crate::widgets::selection_actions::show(
            ui,
            music_rect,
            persisted_pixel_rects.last().copied(),
        ) {
            sel_action = Some(action);
        }
    } else if effective_tool == Tool::Eraser {
        // Draw eraser marquee box in red
        marquee::draw_marquee_box(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            "eraser_drag",
            crate::theme::danger_text_bright(),
            crate::theme::danger_text_bright(),
            false,
        );
    }

    // ── Time ruler ──
    // 横向 = 顶部横条；纵向 = 左侧竖条（时间沿 Y）。
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();

        let ruler_rect = if view.is_vertical() {
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, content_y),
                egui::pos2(rect.min.x + RULER_H, content_bottom),
            )
        } else {
            // 左上角角落（键盘列上方）：量化按钮已移至标尺下方控制栏，此处只补背景。
            let left_corner = egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.min.x + view.keyboard_width(), ruler_band_y + RULER_H),
            );
            ui.painter()
                .rect_filled(left_corner, 0.0, crate::theme::track_bg());

            // 右上角角落：标尺右缘到垂直滚动条之间（SCROLLBAR_W × RULER_H）
            let corner_rect = egui::Rect::from_min_max(
                egui::pos2(content_right_x, ruler_band_y),
                egui::pos2(rect.max.x, ruler_band_y + RULER_H),
            );
            ui.painter()
                .rect_filled(corner_rect, 0.0, crate::theme::track_bg());

            egui::Rect::from_min_max(
                egui::pos2(rect.min.x + view.keyboard_width(), ruler_band_y),
                egui::pos2(content_right_x, ruler_band_y + RULER_H),
            )
        };

        let ruler_jumped = crate::widgets::time_ruler::interactive_ruler(
            ui,
            ruler_rect,
            view,
            tpb,
            def_num,
            def_den,
            sig_events,
            |tick| crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data),
            "piano_ruler",
            cursor_tick,
        );
        // 点击/拖动时间标尺跳转位置时，取消已选择的选框（含框选与全选）。
        if ruler_jumped {
            selected.clear();
            sel_rect.clear();
        }
    }

    // ── PR 控制栏（标尺下方、GPU 画布上方：量化/音轨名称/和弦指示器）──
    {
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, ruler_band_y + RULER_H),
            egui::pos2(rect.max.x, ruler_band_y + RULER_H + theme::PR_BAR_H),
        );
        control_bar::show(ui, bar_rect, &bar, feedback.bar_events);
    }

    // ── Automation panels ──
    let panels_y = content_rect.max.y;
    // 自动化面板的状态栏提示（鼠标在面板 grid_area 内时由 show_panels 写入）
    let mut panels_status_hint: Option<String> = None;
    if let Some(ctx) = auto_ctx.as_mut() {
        let kb_w = view.keyboard_width();
        let combo_w = kb_w * theme::AUTO_PANEL_COMBO_WIDTH_RATIO;

        // automation 编辑上下文：Pencil/Curve/Select/SelectVertical 工具时启用。
        // active_track 由 write_track 决定（与 pencil 一致），但 Conductor 除外：
        // Conductor 不能作为非 Tempo 自动化编辑目标（Tempo 编辑不依赖 active_track，
        // 见 dispatch_edit_interaction）。
        let active_track = write_track
            .filter(|&t| track_visible.get(t as usize).copied().unwrap_or(false))
            .filter(|&t| Some(t) != conductor_idx);
        let edit_ctx = if matches!(
            *active_tool,
            Tool::Pencil | Tool::Curve | Tool::Select | Tool::SelectVertical
        ) {
            Some(automation_panel::AutomationEditCtx {
                active_tool: *active_tool,
                active_track,
                quantize,
                ppq,
                bar_line_data,
            })
        } else {
            None
        };

        let mut panels_state = automation_panel::PanelsState {
            panels: ctx.panels,
            renderers: ctx.renderers,
            wgpu_state: ctx.wgpu_state,
            show_panels: ctx.show,
        };
        let panels_data = automation_panel::PanelsData {
            automation_lanes: ctx.lanes,
            render_lanes: ctx.render_lanes,
            tempo_lane,
            midi,
            track_visible,
            track_colors,
        };
        let panels_layout = automation_panel::PanelsLayout {
            combo_width: combo_w,
            content_rect_right: rect.max.x,
            content_top_y: panels_y,
            panels_visible_h: panels_total_h,
        };
        let panels_cfg = automation_panel::PanelsCfg {
            // AM 面板时间轴始终与 PR 的「时间滚动」同步：横向 = scroll_x，
            // 纵向瀑布流 = scroll_y（面板内部仍横向绘制，时间=X）。
            pianoroll_scroll_x: if view.is_vertical() {
                view.base.scroll_y
            } else {
                view.base.scroll_x
            },
            pianoroll_ppt: view.base.pixels_per_tick,
            scroll_mode,
            min_border_width,
            revision,
            bar_line_data,
            sel_hint,
            // write_track 只在主音轨 = Conductor 时才等于 conductor_idx
            // （无选中时回退到非 Conductor 轨），因此可据此判断。
            editing_is_conductor: write_track == conductor_idx,
        };
        let mut panels_edit = automation_panel::PanelsEdit {
            selected,
            info_content: feedback.info_content,
            right_tab: feedback.right_tab,
        };
        let (_h, auto_edits, velocity_edits, auto_feedback, auto_drag_info) =
            automation_panel::show_panels(
                ui,
                &mut panels_state,
                &panels_data,
                panels_layout,
                panels_cfg,
                &mut panels_edit,
                edit_ctx.as_ref(),
            );
        panels_status_hint = auto_feedback.status_hint.clone();
        for edit in auto_edits {
            feedback.auto_edit_events.push(edit);
        }
        feedback.velocity_edits.extend(velocity_edits);

        // 应用 automation 面板的 pianoroll 联动反馈（主轴滚动/缩放）
        if auto_feedback.scroll_x_delta != 0.0 {
            if view.is_vertical() {
                view.base.scroll_y -= auto_feedback.scroll_x_delta;
            } else {
                view.base.scroll_x -= auto_feedback.scroll_x_delta;
            }
            view.base.dirty = true;
        }
        if (auto_feedback.zoom_factor - 1.0).abs() > 0.001 {
            if view.is_vertical() {
                view.zoom_around_y(
                    auto_feedback.zoom_center_x,
                    auto_feedback.zoom_factor,
                    content_rect.height(),
                );
            } else {
                view.zoom_around_x(auto_feedback.zoom_center_x, auto_feedback.zoom_factor);
            }
        }

        // 存储 ghost drag info 供信息面板实时显示
        *feedback.automation_drag_ghost = auto_drag_info;

        if midi.is_some() {
            let sb_y = rect.min.y + rect.height() - crate::widgets::scrollbar::SCROLLBAR_H;
            let sb_left_blank = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, sb_y),
                egui::pos2(
                    rect.min.x + kb_w,
                    sb_y + crate::widgets::scrollbar::SCROLLBAR_H,
                ),
            );
            ui.painter()
                .rect_filled(sb_left_blank, 0.0, theme::track_bg());
            ui.scope_builder(egui::UiBuilder::new().max_rect(sb_left_blank), |ui| {
                ui.horizontal_centered(|ui| {
                    let mut count = ctx.panels.len();
                    automation_panel::show_toggle_buttons(ui, ctx.show, &mut count);
                    while ctx.panels.len() < count {
                        ctx.panels.push(yinhe_types::AutomationPanelView::default());
                    }
                    while ctx.panels.len() > count {
                        ctx.panels.pop();
                    }
                });
            });
        }
    }

    // ── 滚动条（时间轴 + 音高轴）──
    let sb_y = rect.min.y + rect.height() - crate::widgets::scrollbar::SCROLLBAR_H;
    let pr_orientation = view.orientation();

    // 右下角角落：横纵滚动条交叠区（SCROLLBAR_W × SCROLLBAR_H）
    let corner_rect = egui::Rect::from_min_max(
        egui::pos2(content_right_x, sb_y),
        egui::pos2(rect.max.x, rect.max.y),
    );
    ui.painter()
        .rect_filled(corner_rect, 0.0, crate::theme::track_bg());

    if view.is_vertical() {
        // ── 纵向瀑布流：时间滚动条竖在右侧（绑 scroll_y / ppt），音高滚动条横在底部（绑 scroll_x / key_height）──
        // 时间竖条
        let tick_sb_rect = egui::Rect::from_min_max(
            egui::pos2(content_right_x, content_y),
            egui::pos2(rect.max.x, content_bottom),
        );
        let main_len = content_rect.height();
        let tick_sb_drag = ui
            .push_id("piano_scrollbar", |ui| {
                crate::widgets::scrollbar::show(
                    ui,
                    tick_sb_rect,
                    main_len,
                    &mut view.base.scroll_y,
                    &mut view.base.pixels_per_tick,
                    total_ticks,
                    &mut view.base.dirty,
                    pr_orientation,
                )
            })
            .inner;
        if tick_sb_drag != 0.0 {
            let factor = 1.0 - tick_sb_drag * 0.005;
            let anchor_y = tick_sb_rect.center().y - content_rect.min.y;
            view.zoom_around_y(anchor_y, factor, main_len);
            ui.ctx().request_repaint();
        }

        // 音高横条
        let key_sb_rect = egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, sb_y),
            egui::pos2(content_right_x, rect.max.y),
        );
        let cross_len = content_rect.width();
        let cell_min = cross_len / 128.0;
        let cell_max = cross_len / 12.0;
        let key_sb_drag = ui
            .push_id("piano_vscroll", |ui| {
                crate::widgets::scrollbar::show_vertical(
                    ui,
                    key_sb_rect,
                    cross_len,
                    &mut view.base.scroll_x,
                    &mut view.key_height,
                    128,
                    cell_min,
                    cell_max,
                    &mut view.base.dirty,
                    pr_orientation,
                )
            })
            .inner;
        if key_sb_drag != 0.0 {
            let factor = 1.0 - key_sb_drag * 0.005;
            let anchor_x = key_sb_rect.center().x - content_rect.min.x;
            view.zoom_around_x(anchor_x, factor);
            ui.ctx().request_repaint();
        }

        // 滚动条滚轮缩放：时间条上滚 = 时间缩放（沿 Y）；音高条上滚 = 音高缩放（沿 X）
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
            && !super::view_interaction::pointer_over_popup(ui.ctx())
        {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                if tick_sb_rect.contains(pos) {
                    let anchor_y = tick_sb_rect.center().y - content_rect.min.y;
                    view.zoom_around_y(anchor_y, factor, main_len);
                    ui.ctx().request_repaint();
                } else if key_sb_rect.contains(pos) {
                    let anchor_x = key_sb_rect.center().x - content_rect.min.x;
                    view.zoom_around_x(anchor_x, factor);
                    ui.ctx().request_repaint();
                }
            }
        }
    } else {
        // ── 横向（现状）：时间横条（绑 scroll_x / ppt）+ 音高竖条（绑 scroll_y / key_height）──
        let sb_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + kb_w, sb_y),
            egui::pos2(
                content_right_x,
                sb_y + crate::widgets::scrollbar::SCROLLBAR_H,
            ),
        );
        let sb_drag_dy = ui
            .push_id("piano_scrollbar", |ui| {
                crate::widgets::scrollbar::show(
                    ui,
                    sb_rect,
                    w as f32 - kb_w,
                    &mut view.base.scroll_x,
                    &mut view.base.pixels_per_tick,
                    total_ticks,
                    &mut view.base.dirty,
                    pr_orientation,
                )
            })
            .inner;
        if sb_drag_dy != 0.0 {
            let factor = 1.0 - sb_drag_dy * 0.005;
            let anchor_x = sb_rect.center().x - content_rect.min.x;
            view.zoom_around_x(anchor_x, factor);
            ui.ctx().request_repaint();
        }

        // 音高竖条
        let vsb_rect = egui::Rect::from_min_max(
            egui::pos2(content_right_x, content_y),
            egui::pos2(rect.max.x, content_y + content_h),
        );
        let cell_min = content_rect.height() / 128.0;
        let cell_max = content_rect.height() / 12.0;
        let vsb_drag_dx = ui
            .push_id("piano_vscroll", |ui| {
                crate::widgets::scrollbar::show_vertical(
                    ui,
                    vsb_rect,
                    content_rect.height(),
                    &mut view.base.scroll_y,
                    &mut view.key_height,
                    128,
                    cell_min,
                    cell_max,
                    &mut view.base.dirty,
                    pr_orientation,
                )
            })
            .inner;
        if vsb_drag_dx != 0.0 {
            let factor = 1.0 - vsb_drag_dx * 0.005;
            let anchor_y = vsb_rect.center().y - content_rect.min.y;
            view.zoom_around_y(anchor_y, factor, content_rect.height());
            ui.ctx().request_repaint();
        }

        // 滚动条滚轮缩放：水平滚动条滚轮 = x 轴缩放；垂直滚动条滚轮 = y 轴缩放
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
            && !super::view_interaction::pointer_over_popup(ui.ctx())
        {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                if vsb_rect.contains(pos) {
                    let anchor_y = vsb_rect.center().y - content_rect.min.y;
                    view.zoom_around_y(anchor_y, factor, content_rect.height());
                    ui.ctx().request_repaint();
                } else if sb_rect.contains(pos) {
                    let anchor_x = sb_rect.center().x - content_rect.min.x;
                    view.zoom_around_x(anchor_x, factor);
                    ui.ctx().request_repaint();
                }
            }
        }
    }

    // ── Perf probe: submit per-frame sample ──
    perf::submit(perf::PerfCtx {
        t_show_start,
        t_input_end,
        t_prepare_end,
        t_paint_end,
        follow_mode,
        midi,
        view,
        width: w as f32,
    });

    // ── 状态栏讲解行：钢琴卷帘悬停提示（位置 + 音高）──
    // 自动化面板的提示（grid_area 内）优先于 PR 内容区提示；
    // 鼠标在视图内但不在任何可讲解区域（标尺/滚动条/面板空白）→ 清空。
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && rect.contains(pos)
    {
        let hint = if let Some(h) = panels_status_hint {
            Some(h)
        } else if music_rect.contains(pos) {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            // 主轴/副轴按方向取 local 分量（横向：主轴=x、副轴=y；纵向反之）
            let (main_px, cross_px) =
                crate::selection::drag::main_cross_x_y(view, (local.x, local.y));
            let tick = crate::selection::drag::main_px_to_tick_dir(view, main_px).max(0.0);
            let key = view.cross_px_to_key(cross_px);
            // 本视图有选框 → 讲解行显示选框统计（参考 info panel）
            let sel_text = if !sel_rect.effective_rects().is_empty()
                && let Some(sh) = sel_hint
            {
                Some(t!("hint.sel_notes", n = sh.count, span = &sh.span).to_string())
            } else {
                None
            };
            if let Some(s) = sel_text {
                Some(s)
            } else {
                let pos_str = match bar_line_data {
                    Some((ppq, num, den, events)) => {
                        format_tick_bar_beat_with_time_sig(tick, ppq, events, num, den)
                    }
                    None => format!("{}", tick as u32),
                };
                Some(format!("{} {}", pos_str, key))
            }
        } else if if view.is_vertical() {
            keyboard_rect.contains(pos) // 纵向底部键盘条
        } else {
            content_rect.contains(pos) // 横向键盘列
        } {
            // 键盘区：只显示音高数字（纵向时键盘在底部，音高沿 x）
            let key = if view.is_vertical() {
                view.cross_px_to_key(pos.x - content_rect.min.x)
            } else {
                view.y_to_key(pos.y - content_rect.min.y)
            };
            Some(format!("{}", key))
        } else {
            None
        };
        *feedback.status_hint = hint;
    }

    sel_action
        .map(PianoViewEvent::SelectionAction)
        .or(pencil_event)
        .or(eraser_event)
}
