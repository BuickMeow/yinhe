use super::upload::pr_track_colors;
use super::*;

pub struct AmTarget {
    sink: Sink,
    renderer: InstanceRenderer,
}

impl AmTarget {
    fn new(gpu: &Gpu, sink: Sink) -> Self {
        let format = sink.format();
        Self {
            sink,
            renderer: InstanceRenderer::new_with_caps(
                gpu.device.clone(),
                gpu.queue.clone(),
                format,
                gpu.caps,
            ),
        }
    }

    pub fn window_ptr(&self) -> Option<usize> {
        self.sink.window_ptr()
    }
}

/// 用 AM 面（独立 Flutter 纹理）的原生窗口建立独立 surface 与 renderer。
pub fn create_automation_target(gpu: &Gpu, window: OwnedWindow) -> Option<AmTarget> {
    let sink = Sink::surface(gpu, window, true)?;
    Some(AmTarget::new(gpu, sink))
}

/// iOS：AM 面板的外部纹理呈现端。
#[cfg(target_os = "ios")]
pub fn create_external_automation_target(gpu: &Gpu, kind: i32) -> Option<AmTarget> {
    Some(AmTarget::new(gpu, Sink::external(kind)))
}

/// 渲染一帧 AM 面板：力度条/自动化曲线直接画进独立纹理。
///
/// 主 renderer 的力度条摘要由调用方传入（只读共享），AM renderer 不重建。
pub fn render_automation_frame(
    gpu: &Gpu,
    t: &mut AmTarget,
    summary: Option<Arc<yinhe_wgpu::VelocitySummary>>,
) {
    let (pw, ph) = match crate::automation_pixel_size() {
        (w, h) if w > 0 && h > 0 => (w, h),
        _ => t.sink.size(),
    };
    if t.sink.size() != (pw, ph) {
        t.sink.resize(gpu, pw, ph);
    }
    let (fw, fh) = t.sink.size();

    let snapshot = {
        let Ok(s) = crate::scene::scene().lock() else {
            return;
        };
        let state = s.automation_panel.clone();
        if !state.visible || state.w < 1.0 || state.h < 1.0 {
            return;
        }
        let Some(model) = s.model.clone() else {
            return;
        };
        let visible = {
            let raw = s
                .document
                .as_ref()
                .map(|d| d.edit.pr_visible())
                .unwrap_or_default();
            effective_visible(&raw, s.display_track)
        };
        let track_selected = s
            .document
            .as_ref()
            .map(|d| d.edit.track_selected.clone())
            .unwrap_or_default();
        let track_colors = pr_track_colors(
            &s.track_colors,
            s.active_track,
            &track_selected,
            s.gray_other_tracks,
        );
        let revision = s.model_revision;
        let pr_view = s.pr_view.clone();
        (
            state,
            model,
            visible,
            track_colors,
            track_selected,
            revision,
            pr_view,
        )
    };
    let (state, model, visible, track_colors, track_selected, revision, pr_view) = snapshot;

    let is_velocity = state.lane.is_none();
    let is_conductor = model
        .tracks
        .get(state.active_track)
        .is_some_and(|t| t.kind == TrackKind::Master);
    // Conductor 的 AM 只有 Tempo（不在轨 lane 里，直接取 conductor.tempo）。
    let tempo_lane = if is_conductor {
        Some(model.conductor.tempo.clone())
    } else {
        None
    };
    let lane = if is_conductor {
        if state.lane == Some(0) {
            tempo_lane.as_ref()
        } else {
            None
        }
    } else {
        state.lane.and_then(|idx| {
            model.tracks.get(state.active_track).and_then(|track| {
                track
                    .automation_lanes
                    .iter()
                    .filter(|l| crate::scene::lane_kind(&l.target).is_some())
                    .nth(idx)
            })
        })
    };
    let lanes: Vec<&AutomationLane> = lane.into_iter().collect();
    let value_max = if is_conductor {
        crate::scene::tempo_max_bpm(&model)
    } else {
        1.0
    };
    // Conductor 的拖动 ghost 也传归一化值，绘制前换算成 BPM。
    let ghost_moves: Vec<(u32, u32, f32)> = if is_conductor {
        state
            .ghost
            .iter()
            .map(|&(old, new, value)| (old, new, value * value_max))
            .collect()
    } else {
        state.ghost.clone()
    };
    let ghost = lanes.first().and_then(|lane| {
        if ghost_moves.is_empty() {
            return None;
        }
        let color = track_colors
            .get(state.active_track)
            .map(|c| [c[0], c[1], c[2]])
            .unwrap_or([0.5, 0.5, 0.5]);
        Some(AutomationGhost::Move {
            lane: yinhe_wgpu::build_lane_multi_move(lane, &ghost_moves),
            color,
        })
    });

    let view = AutomationPanelView {
        base: TimelineViewBase {
            pixels_per_tick: pr_view.base.pixels_per_tick,
            scroll_x: pr_view.base.scroll_x,
            scroll_y: 0.0,
            // 纹理从 combo 列右侧开始，局部 x=0 就是网格左缘。
            left_panel_width: 0.0,
            dirty: false,
            track_panel_row_height: 0.0,
            track_panel_scroll_y: 0.0,
            follow_target: None,
            follow_anim_start: 0.0,
            follow_anim_elapsed: 0.0,
        },
        panel_height: state.h,
        selected_target: lanes
            .first()
            .map(|l| l.target.clone())
            .unwrap_or(AutomationTarget::CC { controller: 7 }),
        show_velocity: is_velocity,
        ..Default::default()
    };

    let Some(frame) = t.sink.acquire(gpu) else {
        return;
    };
    let target_view = frame.view.clone();
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("yinhe_automation_frame"),
        });

    t.renderer.set_velocity_summary_shared(summary);

    if !is_velocity && lanes.is_empty() {
        t.renderer.upload_curve_layer(1, 0, |_| {});
        t.renderer.upload_curve_layer(2, 0, |_| {});
    } else {
        yinhe_wgpu::prepare_automation(
            &mut t.renderer,
            1,
            state.w as u32,
            state.h as u32,
            &view,
            &lanes,
            Some(model.as_ref()),
            &visible,
            &track_selected,
            &track_colors,
            1.0,
            state.show_anchors,
            yinhe_wgpu::ANCHOR_SCALE_TOUCH,
            if is_velocity { 127.0 } else { value_max },
            ghost,
            revision,
            &state.selected_ticks,
        );
    }

    t.renderer.draw(&mut encoder, &target_view, fw, fh);
    gpu.queue.submit([encoder.finish()]);
    t.sink.present(gpu, frame);
}
