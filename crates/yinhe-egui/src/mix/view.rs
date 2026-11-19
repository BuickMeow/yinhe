use super::*;

/// MIX 模式主入口（layout.rs 在 Mix 模式且已打开工程时调用）。
pub(crate) fn show(app: &mut App, ui: &mut egui::Ui, rect: egui::Rect) {
    let Some(idx) = app.workspace.active_doc else {
        return;
    };
    crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.mix"));

    // 本帧的只读数据快照（Arc 克隆便宜；layout 与引擎同源，dense 映射一致）。
    let model = app.workspace.documents[idx].data.model.clone();
    app.mix.channel_display = app.workspace.documents[idx].mixer.channel_display_map();
    let layout = ChannelLayout::from_model(&model);
    let active: Vec<u8> = (0..SOURCE_CHANNELS)
        .filter(|&c| layout.is_ap_active(c))
        .map(|c| c as u8)
        .collect();
    // SEND 源：默认选第一条可发送通道，保证旋钮可见；点通道卡片可切换。
    // 源失效（删轨/切文档）时回退到首条；Master 是合法源（表示不可发送）。
    let mut available: Vec<ChannelId> = active
        .iter()
        .filter_map(|&ch| app.workspace.documents[idx].mixer.ap_channel_id(ch))
        .collect();
    available.extend(
        layout
            .track_lc_channels()
            .iter()
            .map(|&(_, lc)| ChannelId::Lc(lc)),
    );
    available.extend(
        layout
            .audio_channels()
            .iter()
            .filter_map(|&ach| app.workspace.documents[idx].mixer.audio_node(ach))
            .map(|node| node.id),
    );
    if !app
        .mix
        .send_source
        .is_some_and(|source| source == ChannelId::Master || available.contains(&source))
    {
        app.mix.send_source = available.first().copied();
    }
    let colors = channel_strip_colors(app, idx, &model, &active);

    let dt = ui.ctx().input(|i| i.stable_dt).min(0.1);
    // 引擎重建后通道数变化 → 重置滑动峰值。
    let channel_count = app
        .audio_state
        .handle
        .as_ref()
        .map(|a| a.handle.mixer_channel_count())
        .unwrap_or(0);
    if app.mix.smoothed.len() != channel_count {
        app.mix.smoothed = vec![MeterState::default(); channel_count];
    }

    let mut actions: Vec<MixAction> = Vec::new();

    // 铺背景。
    ui.painter().rect_filled(rect, 0.0, crate::theme::app_bg());

    // 左侧工具栏（PR 面板工具条同款）：从面板左缘扣出竖条，右侧为混音主体。
    // 工具栏在主体之后绘制（见下方），保证层级高于混音台元素。
    let tool_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.min.x + crate::theme::TOOL_BAR_W, rect.max.y),
    );
    draw_mixer_body(
        app,
        ui,
        rect,
        tool_rect,
        idx,
        &active,
        &colors,
        &layout,
        dt,
        &mut actions,
    );

    // 左侧工具栏：在主体之后绘制（后注册的 widget 命中优先），层级高于混音台
    // 元素；只投递 MixAction、按钮 id 稳定，不存在状态或时序问题。
    tool_bar::show(app, ui, tool_rect, &mut actions);

    // 统一应用本帧动作。
    for action in actions {
        actions::apply_action(app, ui.ctx(), idx, action);
    }

    request_meter_repaint(app, ui);
}

/// 每个通道条的色带颜色：取该通道首个使用轨的颜色（Conductor 不参与）。
pub(crate) fn channel_strip_colors(
    app: &App,
    idx: usize,
    model: &yinhe_core::YinModel,
    active: &[u8],
) -> Vec<egui::Color32> {
    let edit = &app.workspace.documents[idx].edit;
    let track_colors = &edit.track_cache.colors;
    let conductor_idx = edit.track_cache.conductor_idx;
    active
        .iter()
        .map(|&ch| {
            let first_track = model.tracks.iter().enumerate().find_map(|(ti, t)| {
                (Some(ti as u16) != conductor_idx && t.midi_target_channel() == Some(ch))
                    .then_some(ti)
            });
            let c = first_track
                .and_then(|ti| track_colors.get(ti).copied())
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
            crate::theme::rgba_to_color32((c[0], c[1], c[2], c[3]))
        })
        .collect()
}

/// 混音主体：左侧通道条滚动区 + 右侧固定 Master。
#[allow(clippy::too_many_arguments)] // 视图上下文透传
pub(crate) fn draw_mixer_body(
    app: &mut App,
    ui: &mut egui::Ui,
    rect: egui::Rect,
    tool_rect: egui::Rect,
    idx: usize,
    active: &[u8],
    colors: &[egui::Color32],
    layout: &ChannelLayout,
    dt: f32,
    actions: &mut Vec<MixAction>,
) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                egui::pos2(tool_rect.max.x, rect.min.y),
                rect.max,
            ))
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
        |ui| {
            // 主体：左剩余（横向滚动通道条）+ 右固定 Master（贴面板右缘，无间隙）。
            crate::widgets::grid::two_columns_right(ui, 0.0, strip::STRIP_WIDTH, |cells| {
                cells.cell(|ui| {
                    draw_channel_strips(app, ui, idx, active, colors, layout, dt, actions);
                });
                cells.cell(|ui| {
                    draw_master_strip(app, ui, idx, actions);
                });
            });
        },
    );
}

/// 通道条滚动区：MIDI 通道 → 音频通道 → 每轨 LC。
#[allow(clippy::too_many_arguments)] // 视图上下文透传
pub(crate) fn draw_channel_strips(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    active: &[u8],
    colors: &[egui::Color32],
    layout: &ChannelLayout,
    dt: f32,
    actions: &mut Vec<MixAction>,
) {
    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = strip::STRIP_GAP;
            let strip_h = ui.available_height();
            ui.horizontal_top(|ui| {
                for (i, &ch) in active.iter().enumerate() {
                    let dense = layout.dense_for(ch as usize);
                    let (level, hold) = if dense != u32::MAX {
                        smoothed_peak(
                            app.audio_state.handle.as_ref(),
                            &mut app.mix.smoothed,
                            dense as usize,
                            dt,
                        )
                    } else {
                        ((0.0, 0.0), (0.0, 0.0))
                    };
                    strip::channel_strip(
                        app, ui, idx, ch, colors[i], level, hold, strip_h, actions,
                    );
                }
                draw_audio_and_track_strips(app, ui, idx, layout, dt, strip_h, actions);
            });
        });
}

/// 音频通道条与每轨 LC 条。
pub(crate) fn draw_audio_and_track_strips(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    layout: &ChannelLayout,
    dt: f32,
    strip_h: f32,
    actions: &mut Vec<MixAction>,
) {
    draw_audio_strips(app, ui, idx, layout, dt, strip_h, actions);
    draw_track_lc_strips(app, ui, idx, layout, dt, strip_h, actions);
}

/// 音频通道条：MIDI 条后分隔，每个音频通道一条。
fn draw_audio_strips(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    layout: &ChannelLayout,
    dt: f32,
    strip_h: f32,
    actions: &mut Vec<MixAction>,
) {
    let audio_channels: Vec<u16> = layout.audio_channels().to_vec();
    if audio_channels.is_empty() {
        return;
    }
    ui.separator();
    for &ach in audio_channels.iter() {
        let dense = layout.audio_dense_for(ach);
        let (level, hold) = if dense != u32::MAX {
            smoothed_peak(
                app.audio_state.handle.as_ref(),
                &mut app.mix.smoothed,
                dense as usize,
                dt,
            )
        } else {
            ((0.0, 0.0), (0.0, 0.0))
        };
        let Some(node) = app.workspace.documents[idx]
            .mixer
            .audio_node(ach)
            .map(|n| n.id)
        else {
            continue;
        };
        let display = app.mix.channel_display.get(&node).copied().unwrap_or(0);
        strip::audio_strip(app, ui, idx, node, display, level, hold, strip_h, actions);
    }
}

/// 每轨独立 LC 条（Rack per-track 输出）。
fn draw_track_lc_strips(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    layout: &ChannelLayout,
    dt: f32,
    strip_h: f32,
    actions: &mut Vec<MixAction>,
) {
    let track_lcs = layout.track_lc_channels().to_vec();
    if track_lcs.is_empty() {
        return;
    }
    ui.separator();
    for &(track, lc) in &track_lcs {
        let dense = layout.track_lc_dense(track as usize);
        let (level, hold) = if dense != u32::MAX {
            smoothed_peak(
                app.audio_state.handle.as_ref(),
                &mut app.mix.smoothed,
                dense as usize,
                dt,
            )
        } else {
            ((0.0, 0.0), (0.0, 0.0))
        };
        let display = app
            .mix
            .channel_display
            .get(&ChannelId::Lc(lc))
            .copied()
            .unwrap_or(0);
        strip::track_lc_strip(
            app,
            ui,
            idx,
            track,
            ChannelId::Lc(lc),
            display,
            level,
            hold,
            strip_h,
            actions,
        );
    }
}

/// Master 通道条（贴面板右缘）。
pub(crate) fn draw_master_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    actions: &mut Vec<MixAction>,
) {
    let (master_level, master_hold) = app.mix.master_display;
    let rect = ui.max_rect();
    ui.set_clip_rect(rect.intersect(ui.clip_rect()));
    strip::master_strip(
        app,
        ui,
        idx,
        master_level,
        master_hold,
        rect.height(),
        actions,
    );
}

/// 电平表动画：播放中或衰减未归零时保持约 30fps 重绘。
pub(crate) fn request_meter_repaint(app: &App, ui: &egui::Ui) {
    let any_level = app.mix.smoothed.iter().any(|s| {
        let (l, r) = s.level();
        l > 0.001 || r > 0.001
    }) || {
        let (l, r) = app.mix.smoothed_master.level();
        l > 0.001 || r > 0.001
    };
    let playing = app
        .audio_state
        .handle
        .as_ref()
        .is_some_and(|a| a.handle.is_playing());
    if playing || any_level {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
}
