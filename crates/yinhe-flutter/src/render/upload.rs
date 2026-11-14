use super::*;

/// 音轨置顶集合：真实选中轨 + 当前编辑轨（仅用于渲染排序）。
pub(super) fn z_order_track_set(
    track_selected: &HashSet<u16>,
    active_track: Option<usize>,
) -> HashSet<u16> {
    let mut out = track_selected.clone();
    if let Some(t) = active_track {
        out.insert(t as u16);
    }
    out
}

/// PR 用的轨道颜色：开启去色时，仅当前编辑轨保留原色。
pub(super) fn pr_track_colors(
    colors: &[[f32; 4]],
    active_track: Option<usize>,
    track_selected: &HashSet<u16>,
    gray_other_tracks: bool,
) -> Vec<[f32; 4]> {
    if !gray_other_tracks {
        return colors.to_vec();
    }
    // 当前编辑轨 + 所有选中轨都保留彩色，避免多选时把选中的轨也灰掉。
    let keep: Vec<bool> = (0..colors.len())
        .map(|i| active_track == Some(i) || track_selected.contains(&(i as u16)))
        .collect();
    yinhe_editor_core::document::gray_other_track_colors(colors, &keep)
}

/// AR 折叠自动化 lane：只收集可见轨道行的非 conductor lane。
pub(super) fn folded_automation_lanes<'a>(
    model: &'a YinModel,
    view: &ArrangementView,
    visible: &[bool],
    height: f32,
    auto_ghost: &'a [crate::scene::AutoGhostLane],
) -> Vec<yinhe_wgpu::ArrAutomationLane<'a>> {
    let num_tracks = model.tracks.len();
    if num_tracks == 0 {
        return Vec::new();
    }
    let (first, last) = ArrangementView::visible_track_range_static(
        view.base.scroll_y,
        height,
        view.lane_height(),
        num_tracks,
    );
    let lane_h = view.lane_height();
    let mut lanes = Vec::new();
    for i in first..last.min(num_tracks) {
        let track = &model.tracks[i];
        if !visible.get(i).copied().unwrap_or(true) {
            continue;
        }
        if track.kind == TrackKind::Master {
            // Conductor 行：只显示 Tempo 曲线，值域 = 当前最大 Tempo（满高）。
            let lane = &model.conductor.tempo;
            if !lane.events.is_empty() {
                lanes.push(yinhe_wgpu::ArrAutomationLane {
                    lane,
                    y_top: i as f32 * lane_h - view.base.scroll_y,
                    height: lane_h,
                    max_val: crate::scene::tempo_max_bpm(model),
                    highlight_ticks: &[],
                    skip_ticks: &[],
                    show_anchors: false,
                    translucent: true,
                });
            }
            continue;
        }
        for lane in &track.automation_lanes {
            let max_val = if lane.target == AutomationTarget::Tempo {
                lane.events
                    .iter()
                    .map(|e| e.value)
                    .fold(0.0_f32, f32::max)
                    .max(1.0)
            } else {
                1.0
            };
            // 拖动时隐藏源位置被选中的竖线，由 layer 4 的 ghost 画在目标位置。
            let skip_ticks = auto_ghost
                .iter()
                .find(|g| g.src_track == i as u16 && g.target == lane.target)
                .map(|g| g.src_ticks.as_slice())
                .unwrap_or(&[]);
            lanes.push(yinhe_wgpu::ArrAutomationLane {
                lane,
                y_top: i as f32 * lane_h - view.base.scroll_y,
                height: lane_h,
                max_val,
                highlight_ticks: &[],
                skip_ticks,
                show_anchors: false,
                translucent: true,
            });
        }
    }
    lanes
}

/// PR 视图：uniforms + 音符层（含 LOD 摘要）上传。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(super) fn upload_pianoroll(
    t: &mut Target,
    model: &YinModel,
    view: &PianoRollView,
    track_colors: &[[f32; 4]],
    track_visible: &[bool],
    display_track: Option<usize>,
    active_track: Option<usize>,
    gray_other_tracks: bool,
    selection: &Selection,
    track_selected: &HashSet<u16>,
    model_epoch: u64,
    w: f32,
    h: f32,
    preview: &crate::scene::DragPreview,
    preview_offset: (f32, f32),
) {
    let visible = effective_visible(track_visible, display_track);
    let colors = pr_track_colors(
        track_colors,
        active_track,
        track_selected,
        gray_other_tracks,
    );
    // 当前编辑轨必须置顶：track_selected 只包含“有选中音符的轨”，
    // 从顶部栏切轨时它可能为空；这里把 active_track 并入渲染专用集合，
    // 不改文档里的真实选择集。
    let z_selected = z_order_track_set(track_selected, active_track);
    let mut job = build_render_job(w as u32, h as u32, view, selection, &colors, 0.0, true);
    job.uniforms.ghost_dx = preview_offset.0;
    job.uniforms.ghost_dy = preview_offset.1;
    t.renderer.upload_uniforms(job.uniforms);
    t.renderer.upload_track_colors(&job.track_colors);
    t.renderer.upload_selection(&job.selection);

    let changed = t.pr_note_cache.update(&yinhe_wgpu::NoteCacheParams {
        midi: model,
        view,
        width: w,
        height: h,
        track_visible: &visible,
        track_selected: &z_selected,
        hidden_notes: preview.hidden.as_ref(),
        hidden_hash: preview.hidden_hash,
        selection: Some(selection),
        note_revisions: &model.note_revisions,
        lod_enabled: true,
        source_tag: model_epoch,
    });
    let key = t.pr_note_cache.cache_key();
    let _ = changed;
    t.renderer.ensure_layers(3);
    let cache = &t.pr_note_cache;
    t.renderer.upload_note_layer(1, key, |out| {
        out.extend_from_slice(cache.instances());
    });

    // ghost 层：独立强制更新键（preview.revision 每次 ghost 变化自增）。
    // 结束时即使 ghost 为空也会上传一次空层，清掉上一帧预览。
    let ghost_key = yinhe_wgpu::layer_cache_key(&[
        VIEW_TAG_PIANOROLL ^ 0xA5A5_5A5A,
        preview.revision,
        u64::from(preview.selected),
        model_epoch,
    ])
    .max(1);
    let theme = yinhe_theme::current_gpu_theme();
    t.renderer.upload_note_layer(2, ghost_key, |out| {
        for &(start, end, key, track) in preview.ghost.iter() {
            yinhe_wgpu::build_ghost_note(out, start, end, key, track, preview.selected, &theme);
        }
    });

    // PR 不显示 AR 折叠自动化；上传空 layer 3 清掉 AR 切过来时的残留。
    let am_key = yinhe_wgpu::layer_cache_key(&[
        VIEW_TAG_PIANOROLL ^ 0x51_51_51_51,
        u64::from(w.to_bits()),
        u64::from(h.to_bits()),
    ]);
    yinhe_wgpu::prepare_folded_automation(
        &mut t.renderer,
        w,
        h,
        &view.base,
        &[],
        &visible,
        track_colors,
        tc::bg(),
        tc::stripe_bg(),
        am_key,
    );
    // 清掉 AR 拖动留下的自动化竖线 ghost（layer 4）。
    yinhe_wgpu::prepare_folded_automation_ghost(&mut t.renderer, w, &view.base, &[], &[], &[]);
}

/// AR 视图：uniforms + 轨道布局（offsets/行高）+ 音符层上传。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(super) fn upload_arrangement(
    t: &mut Target,
    model: &YinModel,
    view: &ArrangementView,
    track_colors: &[[f32; 4]],
    track_visible: &[bool],
    display_track: Option<usize>,
    num_tracks: usize,
    revision: u64,
    model_epoch: u64,
    w: f32,
    h: f32,
    preview: &crate::scene::DragPreview,
    preview_offset: (f32, f32),
) {
    let num_tracks = num_tracks.min(yinhe_wgpu::MAX_TRACKS);
    if num_tracks == 0 {
        return;
    }
    let visible = effective_visible(track_visible, display_track);
    let lane_h = view.lane_height();
    let offsets: Vec<f32> = (0..num_tracks).map(|i| i as f32 * lane_h).collect();
    // ★ shader 用 row_heights 计算行高；不传会退化成 1px 高的音符。
    let heights: Vec<f32> = vec![lane_h; num_tracks];
    t.renderer.upload_track_offsets(&offsets);
    t.renderer.upload_row_heights(&heights);
    t.renderer.upload_track_colors(track_colors);

    let uniforms = yinhe_wgpu::Uniforms {
        width: w,
        height: h,
        scroll_x: view.base.scroll_x,
        scroll_y: view.base.scroll_y,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0, // AR 未使用
        keyboard_width: view.base.left_panel_width,
        mode: 2, // AR notes
        min_border_width: 0.0,
        track_count: num_tracks as u32,
        sel_rect_count: 0,
        note_outline: 0,
        lane_height: lane_h,
        value_zoom: 0.0,
        value_scroll: 0.0,
        orientation: 0,
        ghost_dx: preview_offset.0,
        ghost_dy: preview_offset.1,
        // AR lane 纵向音域：项目含扩展音域音符时 256，否则 128。
        key_count: model.display_key_count() as u32,
        ..Default::default()
    };
    t.renderer.upload_uniforms(uniforms);
    // 空选区：防止从 PR 切过来时残留高亮。
    t.renderer
        .upload_selection(&yinhe_wgpu::vertex::SelectionUniform {
            rects: [[0; 4]; yinhe_wgpu::vertex::MAX_SEL_RECTS * 2],
        });

    let mut key = view.render_hash();
    key ^= (w.to_bits() as u64) << 17;
    key ^= (h.to_bits() as u64) << 29;
    key ^= revision.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    key ^= model_epoch.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    key ^= model.note_count.rotate_left(11);
    key ^= preview.hidden_hash.wrapping_mul(0x94D0_49BB_1331_11EB);
    for (i, vis) in visible.iter().enumerate() {
        if *vis {
            key ^= 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1);
        }
    }
    let key = yinhe_wgpu::layer_cache_key(&[key, VIEW_TAG_ARRANGEMENT]);
    if key != t.notes_cache_key {
        t.notes_buf.clear();
        // 只构建当前可见轨道行：undo/Riff 后 AR 层重建不再扫全工程所有轨。
        let (trk_first, trk_last) = ArrangementView::visible_track_range_static(
            view.base.scroll_y,
            h,
            view.lane_height(),
            num_tracks,
        );
        yinhe_wgpu::build_arr_notes(
            &mut t.notes_buf,
            w,
            model,
            view,
            (trk_first, trk_last),
            &visible,
            preview.hidden.as_ref(),
        );
        t.notes_cache_key = key;
    }
    t.renderer.ensure_layers(3);
    let notes_buf = &t.notes_buf;
    t.renderer
        .upload_note_layer(1, key, |out| out.extend_from_slice(notes_buf));
    // AR ghost：与 PR 共用 DragPreview；无 ghost 时上传空层清掉上一帧。
    let ghost_key = yinhe_wgpu::layer_cache_key(&[
        VIEW_TAG_ARRANGEMENT ^ 0xA5A5_5A5A,
        preview.revision,
        model_epoch,
    ])
    .max(1);
    let mut ghost = preview.ghost.as_ref().clone();
    t.renderer.upload_note_layer(2, ghost_key, |out| {
        yinhe_wgpu::build_ghost_notes(
            out,
            ghost.as_mut_slice(),
            w,
            view,
            (0, num_tracks),
            &visible,
        );
        for inst in out.iter_mut() {
            inst.set_ghost(true);
        }
    });

    // 折叠自动化竖线：只给可见轨道、只生成可见 tick 的事件，画在 layer 3。
    let lanes = folded_automation_lanes(model, view, &visible, h, preview.auto_ghost.as_slice());
    let mut am_key = view.render_hash();
    am_key ^= (w.to_bits() as u64) << 17;
    am_key ^= (h.to_bits() as u64) << 29;
    am_key ^= revision.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    am_key ^= model_epoch.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    yinhe_wgpu::prepare_folded_automation(
        &mut t.renderer,
        w,
        h,
        &view.base,
        &lanes,
        &visible,
        track_colors,
        tc::bg(),
        tc::stripe_bg(),
        yinhe_wgpu::layer_cache_key(&[am_key, VIEW_TAG_ARRANGEMENT]),
    );

    // 拖动 ghost：选中自动化锚点按拖动量移动后的竖线（layer 4）。
    let mut ghost_owners: Vec<AutomationLane> = Vec::with_capacity(preview.auto_ghost.len());
    for g in preview.auto_ghost.iter() {
        ghost_owners.push(AutomationLane {
            target: g.target.clone(),
            track: g.track,
            events: g.events.clone(),
        });
    }
    let mut ghost_lanes: Vec<yinhe_wgpu::ArrAutomationLane<'_>> =
        Vec::with_capacity(ghost_owners.len());
    for (g, lane) in preview.auto_ghost.iter().zip(&ghost_owners) {
        ghost_lanes.push(yinhe_wgpu::ArrAutomationLane {
            lane,
            y_top: f32::from(g.track) * lane_h - view.base.scroll_y,
            height: lane_h,
            max_val: g.max_val,
            highlight_ticks: &[],
            skip_ticks: &[],
            show_anchors: false,
            translucent: true,
        });
    }
    yinhe_wgpu::prepare_folded_automation_ghost(
        &mut t.renderer,
        w,
        &view.base,
        &ghost_lanes,
        &visible,
        track_colors,
    );
}
