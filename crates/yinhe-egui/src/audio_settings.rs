pub use yinhe_editor_core::audio_settings::{AudioSettings, Interpolation, SynthEngine};

// `list_output_devices` / `discover_sample_rates` 由 yinhe-audio 统一导出，避免在 yinhe-egui 里再写一份 cpal
// 枚举逻辑（设备切换对话框和设置面板都用这一个）。
pub(crate) use yinhe_audio::{discover_sample_rates, list_input_devices, list_output_devices};

/// Load AudioSettings and populate device lists from the system.
pub(crate) fn load_audio_settings() -> AudioSettings {
    let mut settings = AudioSettings::load();
    // 右栏布局迁移：旧配置缺少后来新增的选项卡（如「图层」）时补齐。
    if settings.layout.right_panel_layout.ensure_all() {
        settings.save();
    }
    // locale 名迁移：旧的非区域代码（en/ja/ko）统一为新格式，避免回退到中文。
    // 已删除的小语种（`locales/` 未提供）→ 回退到 zh-CN（rust-i18n fallback）。
    let legacy = match settings.locale.as_str() {
        "en" => Some("en-US"),
        "ja" => Some("ja-JP"),
        "ko" => Some("ko-KR"),
        "bo-CN" | "fil-PH" | "hi-IN" | "id-ID" | "km-KH" | "lo-LA" | "ms-MY" | "my-MM"
        | "pt-PT" | "th-TH" | "uk-UA" | "vi-VN" => Some("zh-CN"),
        _ => None,
    };
    if let Some(code) = legacy {
        settings.locale = code.to_string();
        settings.save();
    }
    let devices = list_output_devices();
    let (_default_rate, rates) = discover_sample_rates();
    // 上次选的**具体**设备不在当前设备列表里（耳机拔了/换了电脑）→ 回退到「默认设备」
    // 并落盘。判定细节见 `fallback_dangling_output_device`。
    if fallback_dangling_output_device(&mut settings, &devices) {
        settings.save();
    }
    settings.refresh_devices(devices, rates);
    refresh_midi_inputs(&mut settings);
    // 录音输入设备：刷新列表；上次选的设备已拔出时清空（回退系统默认）。
    settings.ui_session.available_input_devices = list_input_devices();
    if let Some(name) = settings.input_device_name.as_ref()
        && !settings
            .ui_session
            .available_input_devices
            .iter()
            .any(|d| d == name)
    {
        settings.input_device_name = None;
    }
    settings
}

/// 悬空输出设备回退：`Some(name)` 且 `name` 不在当前设备列表里（设备被拔出/换机）
/// 时回退到 `None`（=「默认设备」/自动跟随系统），返回是否发生了回退。
///
/// **`None` 永远保持 `None`**，不能固化成具体设备名：`None` 是「自动跟随系统默认」
/// 的有效设置，一旦被固化成 `Some(...)`，`app/dialog_dispatch.rs` 里
/// `using_default_device = output_device_name.is_none()`（约 47 行）恒为 false，
/// 设备列表变更 / 流死亡时的静默 `respawn_audio()` 分支（约 49、71 行）永远走不到，
/// 用户每次插拔设备都会被弹「音频设备切换」对话框。
pub(crate) fn fallback_dangling_output_device(
    settings: &mut AudioSettings,
    devices: &[String],
) -> bool {
    let Some(name) = settings.output_device_name.as_ref() else {
        return false;
    };
    if devices.iter().any(|d| d == name) {
        return false;
    }
    settings.output_device_name = None;
    true
}

/// 枚举系统 MIDI 输入端口并同步到设置；上次选的设备已拔出时清空选择。
pub(crate) fn refresh_midi_inputs(settings: &mut AudioSettings) {
    settings.ui_session.available_midi_inputs =
        yinhe_midi_io::list_input_ports().unwrap_or_default();
    if let Some(name) = settings.midi_input_device.as_ref()
        && !settings
            .ui_session
            .available_midi_inputs
            .iter()
            .any(|d| d == name)
    {
        settings.midi_input_device = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `None`（默认/自动模式）是有效设置：不判定为回退，更不能被固化成具体设备名
    /// （固化后 `app/dialog_dispatch.rs` 的静默重连分支失效，见函数注释）。
    #[test]
    fn none_output_device_stays_none() {
        let mut settings = AudioSettings::default();
        assert!(settings.output_device_name.is_none());
        let fell_back = fallback_dangling_output_device(&mut settings, &["存在".to_string()]);
        assert!(!fell_back, "None 不应触发回退");
        assert!(settings.output_device_name.is_none(), "None 必须保持 None");
    }

    /// 上次选的设备已不在列表（拔出/换机）→ 回退到「默认设备」（None）。
    #[test]
    fn dangling_device_falls_back_to_none() {
        let mut settings = AudioSettings {
            output_device_name: Some("不存在".to_string()),
            ..AudioSettings::default()
        };
        let fell_back = fallback_dangling_output_device(&mut settings, &["存在".to_string()]);
        assert!(fell_back, "悬空设备应触发回退");
        assert!(
            settings.output_device_name.is_none(),
            "回退目标是「默认设备」"
        );
    }

    /// 设备仍在列表里 → 保持用户选择，不回退。
    #[test]
    fn present_device_keeps_selection() {
        let mut settings = AudioSettings {
            output_device_name: Some("存在".to_string()),
            ..AudioSettings::default()
        };
        let fell_back = fallback_dangling_output_device(&mut settings, &["存在".to_string()]);
        assert!(!fell_back, "设备仍存在不应回退");
        assert_eq!(settings.output_device_name.as_deref(), Some("存在"));
    }
}
