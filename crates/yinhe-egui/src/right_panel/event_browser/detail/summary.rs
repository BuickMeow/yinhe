//! Overview / Track 详情与 automation 分类统计。

use yinhe_types::AutomationLane;
use yinhe_types::automation::{ParamDevice, xsynth_param};

use super::*;

// ── Overview / Track detail ──

/// automation lane 分类统计（overview 与 track_detail 共用）。
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct TargetSummary {
    pub(super) cc_total: usize,
    /// `(controller, 事件数)`，按首次出现顺序排列。
    pub(super) cc_per_controller: Vec<(u8, usize)>,
    pub(super) pb_total: usize,
    pub(super) param_total: usize,
    pub(super) rpn_total: usize,
}

impl TargetSummary {
    /// 累加一条 lane 的事件计数（设备参数中 Pitch Bend 单独归类）。
    fn add_lane(&mut self, lane: &AutomationLane) {
        let n = lane.events.len();
        match &lane.target {
            AutomationTarget::CC { controller } => {
                self.cc_total += n;
                match self
                    .cc_per_controller
                    .iter_mut()
                    .find(|(c, _)| c == controller)
                {
                    Some((_, count)) => *count += n,
                    None => self.cc_per_controller.push((*controller, n)),
                }
            }
            AutomationTarget::Param {
                device: ParamDevice::ChannelInstrument { .. },
                id,
                ..
            } if *id == xsynth_param::PITCH_BEND => self.pb_total += n,
            AutomationTarget::PitchBend => self.pb_total += n,
            AutomationTarget::Param { .. } => self.param_total += n,
            AutomationTarget::Rpn { .. } | AutomationTarget::Nrpn { .. } => self.rpn_total += n,
            AutomationTarget::Tempo => {}
        }
    }

    /// 合并另一份统计（CC 分组保持首次出现顺序）。
    pub(super) fn merge(&mut self, other: Self) {
        self.cc_total += other.cc_total;
        for (controller, count) in other.cc_per_controller {
            match self
                .cc_per_controller
                .iter_mut()
                .find(|(c, _)| *c == controller)
            {
                Some((_, total)) => *total += count,
                None => self.cc_per_controller.push((controller, count)),
            }
        }
        self.pb_total += other.pb_total;
        self.param_total += other.param_total;
        self.rpn_total += other.rpn_total;
    }
}

pub(super) fn summarize_targets(lanes: &[AutomationLane]) -> TargetSummary {
    let mut out = TargetSummary::default();
    for lane in lanes {
        out.add_lane(lane);
    }
    out
}

pub(crate) fn show_overview(ui: &mut egui::Ui, model: &yinhe_core::YinModel) {
    crate::right_panel::region_hint(ui, t!("hint.panel.event_overview"));
    let name = if model.meta.name.is_empty() {
        t!("event_browser.meta.untitled").to_string()
    } else {
        model.meta.name.clone()
    };
    let artist = if model.meta.artist.is_empty() {
        t!("event_browser.meta.unfilled").to_string()
    } else {
        model.meta.artist.clone()
    };
    crate::widgets::rows::value_row(ui, t!("event_browser.meta.name"), name);
    crate::widgets::rows::value_row(ui, t!("event_browser.meta.artist"), artist);
    crate::widgets::rows::value_row(ui, "PPQ:", format!("{}", model.meta.ppq));
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.zstd_level"),
        format!("{}", model.meta.compression_level),
    );
    let groups = super::super::group_tracks_by_port_channel(model, None);
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.active_ports"),
        format!("{}", groups.len()),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.tracks"),
        t!("event_browser.count", n = model.tracks.len()),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.notes"),
        t!("event_browser.count", n = model.note_count),
    );
    let mut summary = TargetSummary::default();
    let mut pc = 0usize;
    for t in &model.tracks {
        summary.merge(summarize_targets(&t.automation_lanes));
        pc += t.program_change.len();
    }
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.cc"),
        t!("event_browser.count", n = summary.cc_total),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.params"),
        t!("event_browser.count", n = summary.param_total),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.pitch_bend"),
        t!("event_browser.count", n = summary.pb_total),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.program_change"),
        t!("event_browser.count", n = pc),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.tempo"),
        t!(
            "event_browser.count",
            n = model.conductor.tempo.events.len()
        ),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.timesig"),
        t!("event_browser.count", n = model.conductor.time_sig.len()),
    );
    if !model.conductor.key_sig.is_empty() {
        crate::widgets::rows::value_row(
            ui,
            t!("event_browser.meta.keysig"),
            t!("event_browser.count", n = model.conductor.key_sig.len()),
        );
    }
    if !model.conductor.markers.is_empty() {
        crate::widgets::rows::value_row(
            ui,
            t!("event_browser.meta.markers"),
            t!("event_browser.count", n = model.conductor.markers.len()),
        );
    }
}

pub(crate) fn show_track_detail(
    ui: &mut egui::Ui,
    idx: u16,
    track: &yinhe_core::TrackData,
    model: &yinhe_core::YinModel,
) {
    crate::right_panel::region_hint(ui, t!("hint.panel.event_track_detail"));
    let header = if track.name.is_empty() {
        t!("event_browser.track_unnamed", n = idx).to_string()
    } else {
        t!("event_browser.track_named", n = idx, name = &track.name).to_string()
    };
    crate::widgets::rows::section_header(ui, &header);

    kv(ui, "UUID", track.uuid.clone());
    kv(
        ui,
        "Port / Channel",
        format!(
            "{} / {}",
            super::super::port_letter(track.port),
            track.channel + 1
        ),
    );
    kv(
        ui,
        t!("event_browser.channel_prefix").as_ref(),
        match track.channel_prefix {
            Some(c) => format!("{}", c),
            None => t!("common.none").to_string(),
        },
    );
    kv(
        ui,
        t!("event_browser.color").as_ref(),
        format!(
            "[{:.2}, {:.2}, {:.2}]",
            track.color[0], track.color[1], track.color[2]
        ),
    );
    kv(
        ui,
        t!("event_browser.muted_soloed").as_ref(),
        format!("{} / {}", track.muted, track.soloed),
    );
    ui.add_space(crate::theme::GAP_SM);
    ui.label(
        egui::RichText::new(t!("event_browser.event_counts"))
            .strong()
            .size(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::BODY_FONT,
            ))
            .color(crate::theme::text_bright()),
    );
    kv(
        ui,
        "Notes",
        format!(
            "{}",
            model
                .track_note_count
                .get(idx as usize)
                .copied()
                .unwrap_or(0)
        ),
    );
    // 按 automation target 类型汇总（与 overview 共用同一分类逻辑）
    let summary = summarize_targets(&track.automation_lanes);
    if !summary.cc_per_controller.is_empty() {
        kv(
            ui,
            "CC",
            t!(
                "event_browser.cc_summary",
                controllers = summary.cc_per_controller.len(),
                events = summary.cc_total
            )
            .to_string(),
        );
        for (ctrl, count) in &summary.cc_per_controller {
            kv(
                ui,
                &format!(
                    "  {}",
                    AutomationTarget::CC { controller: *ctrl }.display_name()
                ),
                t!("event_browser.cc_count", n = count).to_string(),
            );
        }
    }
    kv(ui, "Pitch Bend", format!("{}", summary.pb_total));
    if summary.param_total > 0 {
        kv(
            ui,
            t!("event_browser.params").as_ref(),
            format!("{}", summary.param_total),
        );
    }
    kv(
        ui,
        "Program Change",
        format!("{}", track.program_change.len()),
    );
    if summary.rpn_total > 0 {
        kv(
            ui,
            "RPN/NRPN",
            t!("event_browser.rpn_summary", n = summary.rpn_total).to_string(),
        );
    }
}
