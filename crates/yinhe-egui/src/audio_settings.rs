use std::collections::BTreeSet;

use cpal::traits::{DeviceTrait, HostTrait};

pub use yinhe_editor_core::audio_settings::AudioSettings;

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

    let supported_rates: Vec<u32> = device
        .supported_output_configs()
        .ok()
        .map(|configs| {
            configs
                .flat_map(|cfg| {
                    let min = cfg.min_sample_rate();
                    let max = cfg.max_sample_rate();
                    (min..=max).step_by(1000)
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default();

    if supported_rates.is_empty() {
        (default_rate, vec![default_rate])
    } else {
        (default_rate, supported_rates)
    }
}

pub(crate) fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|d| d.description().ok().map(|desc| desc.to_string())).collect::<Vec<_>>())
        .unwrap_or_default()
}

/// Load AudioSettings and populate device lists from the system.
pub(crate) fn load_audio_settings() -> AudioSettings {
    let mut settings = AudioSettings::load();
    let devices = list_output_devices();
    let (default_rate, rates) = discover_sample_rates();
    // Set default device if not already set
    if settings.output_device_name.is_none() {
        settings.output_device_name = cpal::default_host()
            .default_output_device()
            .and_then(|d| d.description().ok().map(|desc| desc.to_string()));
    }
    settings.refresh_devices(devices, rates, default_rate);
    settings
}
