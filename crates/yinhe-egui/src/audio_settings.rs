use cpal::traits::{DeviceTrait, HostTrait};

pub use yinhe_editor_core::audio_settings::AudioSettings;

// `list_output_devices` 由 yinhe-audio 统一导出，避免在 yinhe-egui 里再写一份 cpal
// 枚举逻辑（设备切换对话框和设置面板都用这一个）。
pub(crate) use yinhe_audio::list_output_devices;

/// Query the default output device for its default sample rate and all
/// supported sample rates. Falls back to `(48000, [44100, 48000, 96000])`
/// when no device is available.
pub(crate) fn discover_sample_rates() -> (u32, Vec<u32>) {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        return (48000, vec![44100, 48000, 96000]);
    };

    let default_rate = device
        .default_output_config()
        .ok()
        .map(|cfg| cfg.sample_rate())
        .unwrap_or(48000);

    // 只列标准采样率与设备支持范围的交集，而非按 1000Hz 步进枚举
    // （会生成 45100、46100 等设备实际不支持的值，用户选了会建流失败）。
    const STANDARD_RATES: [u32; 8] = [22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000];
    let supported_rates: Vec<u32> = device
        .supported_output_configs()
        .ok()
        .map(|configs| {
            let ranges: Vec<(u32, u32)> = configs
                .map(|cfg| (cfg.min_sample_rate(), cfg.max_sample_rate()))
                .collect();
            STANDARD_RATES
                .iter()
                .copied()
                .filter(|&rate| ranges.iter().any(|(min, max)| rate >= *min && rate <= *max))
                .collect()
        })
        .unwrap_or_default();

    if supported_rates.is_empty() {
        (default_rate, vec![default_rate])
    } else {
        (default_rate, supported_rates)
    }
}

/// Load AudioSettings and populate device lists from the system.
pub(crate) fn load_audio_settings() -> AudioSettings {
    let mut settings = AudioSettings::load();
    // locale 名迁移：旧的非区域代码（en/ja/ko）统一为新格式，避免回退到中文。
    let legacy = match settings.locale.as_str() {
        "en" => Some("en-US"),
        "ja" => Some("ja-JP"),
        "ko" => Some("ko-KR"),
        _ => None,
    };
    if let Some(code) = legacy {
        settings.locale = code.to_string();
        settings.save();
    }
    let devices = list_output_devices();
    let (default_rate, rates) = discover_sample_rates();
    // 上次选的设备不在当前设备列表里（耳机拔了/换了电脑）→ 回退到系统默认
    let need_default = settings
        .output_device_name
        .as_ref()
        .map(|name| !devices.iter().any(|d| d == name))
        .unwrap_or(true);
    if need_default {
        settings.output_device_name = cpal::default_host()
            .default_output_device()
            .and_then(|d| d.description().ok().map(|desc| desc.to_string()));
        settings.save();
    }
    settings.refresh_devices(devices, rates, default_rate);
    settings
}
