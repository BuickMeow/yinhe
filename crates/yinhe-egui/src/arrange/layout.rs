/// 构建 AR 行布局（含展开的 AM 子行）。
pub(super) fn build_row_layout(
    edit: &yinhe_editor_core::edit_state::EditState,
    model: &yinhe_core::YinModel,
    num_tracks: usize,
    conductor_idx: Option<u16>,
    track_mixer_lanes: &[Vec<(yinhe_types::MixerNodeId, Vec<yinhe_types::AutomationLane>)>],
) -> yinhe_types::ArRowLayout {
    let counts: Vec<u32> = (0..num_tracks)
        .map(|i| {
            if edit.arr_am_expanded.get(i).copied().unwrap_or(false)
                && Some(i as u16) != conductor_idx
            {
                model
                    .tracks
                    .get(i)
                    .map(|t| {
                        t.automation_lanes.len()
                            + track_mixer_lanes
                                .get(i)
                                .map(|lanes| lanes.iter().map(|(_, l)| l.len()).sum::<usize>())
                                .unwrap_or(0)
                    })
                    .unwrap_or(0) as u32
            } else {
                0
            }
        })
        .collect();
    let visible = model.folder_visible_mask();
    yinhe_types::ArRowLayout::new_with_visible(&counts, &visible)
}
