//! 走带控制：播放/暂停/停止/跟随/BPM/位置时间显示（AR 与 PR 顶栏共用）。
//!
//! 以独立 free function 形式提供（`bar` 绘制 UI、`update` 每帧同步播放位置），
//! 避免把 UI 绑死在某个页面模块上。

use eframe::egui;
use yinhe_audio::spawn::AudioCommand;

use crate::app::YinheApp;
use crate::ui_common::icon_text;

/// 走带控制条：播放/暂停、停止、跟随播放开关、BPM、位置/时间显示。
/// 位置 = 小节.拍.tick、时间 = m:ss.mmm（桌面端 timecode 同款格式），
/// 点击位置/时间在两者间切换。
pub(crate) fn bar(app: &mut YinheApp, ui: &mut egui::Ui) {
    if app.model.is_none() {
        ui.label("未加载工程");
        return;
    }
    let Some(audio) = &app.audio else {
        ui.label("音频未初始化");
        return;
    };
    let playing = audio.handle.is_playing();
    use egui_material_icons::icons::{ICON_PAUSE, ICON_PLAY_ARROW, ICON_STOP};
    let play_icon = if playing { ICON_PAUSE } else { ICON_PLAY_ARROW };
    if ui
        .button(icon_text(play_icon))
        .on_hover_text("播放/暂停")
        .clicked()
    {
        if playing {
            audio.handle.send(AudioCommand::Pause);
        } else {
            let from_sample = (app
                .model
                .as_ref()
                .map(|m| m.tempo_map.tick_to_seconds(app.cursor_tick as u64))
                .unwrap_or(0.0)
                * audio.sample_rate as f64) as u64;
            audio.handle.send(AudioCommand::Play { from_sample });
        }
    }
    if ui
        .button(icon_text(ICON_STOP))
        .on_hover_text("停止")
        .clicked()
    {
        audio.handle.send(AudioCommand::Stop);
        app.cursor_tick = 0.0;
        app.pr_view.set_cursor(Some(0.0));
        app.ar_view.set_cursor(Some(0.0));
    }
    // 跟随播放：图标按钮，选中高亮。
    use egui_material_icons::icons::ICON_CENTER_FOCUS_STRONG;
    if ui
        .add(egui::Button::new(icon_text(ICON_CENTER_FOCUS_STRONG)).selected(app.follow_play))
        .on_hover_text(if app.follow_play {
            "跟随播放：开"
        } else {
            "跟随播放：关"
        })
        .clicked()
    {
        app.follow_play = !app.follow_play;
    }
    // 工具按钮（显示当前工具图标，点击弹出居中工具选择窗）。
    // 位置：跟随之后、BPM 之前。
    if ui
        .button(icon_text(app.tool.icon()))
        .on_hover_text(format!("工具：{}", app.tool.name()))
        .clicked()
    {
        app.tool_picker_open = !app.tool_picker_open;
    }

    let Some(model) = &app.model else {
        return;
    };
    let tm = &model.tempo_map;
    // BPM：当前光标处的速度（tempo 分段变化时随位置更新）。
    let cur_sec = tm.tick_to_seconds(app.cursor_tick as u64);
    ui.label(format!(
        "{} BPM",
        yinhe_types::time_format::format_bpm(tm.bpm_at_time(cur_sec))
    ));
    // 位置（小节.拍.tick）与时间（m:ss.mmm）：点击切换显示。
    let (def_num, def_den) = tm.time_sig_default;
    let pos_str = yinhe_types::time_format::format_tick_bar_beat_with_time_sig(
        app.cursor_tick,
        model.meta.ppq,
        &tm.time_sig_events,
        def_num,
        def_den,
    );
    let time_str = yinhe_types::time_format::format_time(cur_sec);
    let time_resp = ui
        .add(
            egui::Label::new(if app.time_show_ticks {
                pos_str
            } else {
                time_str
            })
            .sense(egui::Sense::click()),
        )
        .on_hover_text(if app.time_show_ticks {
            "时间 (秒)".to_string()
        } else {
            "位置 (小节.拍.tick)".to_string()
        });
    if time_resp.clicked() {
        app.time_show_ticks = !app.time_show_ticks;
    }
}

/// 每帧从音频引擎同步播放位置：换算 tick 更新播放光标，跟随模式时滚动视口。
pub(crate) fn update(app: &mut YinheApp) {
    let Some(audio) = &app.audio else {
        return;
    };
    if !audio.handle.is_playing() {
        return;
    }
    let Some(model) = &app.model else {
        return;
    };
    let seconds = audio.handle.sample_position() as f64 / audio.sample_rate as f64;
    let tick = crate::seconds_to_tick(model, seconds);
    app.cursor_tick = tick;
    app.pr_view.set_cursor(Some(tick));
    app.ar_view.set_cursor(Some(tick));
    if app.follow_play {
        app.pr_view.follow_cursor();
        app.ar_view.follow_cursor();
    }
}
