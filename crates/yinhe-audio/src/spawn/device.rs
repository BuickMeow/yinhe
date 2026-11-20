use super::*;

pub(crate) enum DeviceDirection {
    Input,
    Output,
}

pub(crate) fn list_devices(direction: DeviceDirection) -> Vec<String> {
    audio_out::list_devices(matches!(direction, DeviceDirection::Output))
}

pub fn list_output_devices() -> Vec<String> {
    list_devices(DeviceDirection::Output)
}

pub fn list_input_devices() -> Vec<String> {
    list_devices(DeviceDirection::Input)
}

pub fn discover_sample_rates() -> (u32, Vec<u32>) {
    let Some(device) = audio_out::default_output_device() else {
        return (48000, vec![44100, 48000, 96000]);
    };

    let default_rate = device
        .default_format()
        .map(|fmt| fmt.sample_rate)
        .unwrap_or(48000);

    // 只列标准采样率与设备支持范围的交集，而非按 1000Hz 步进枚举
    // （会生成 45100、46100 等设备实际不支持的值，用户选了会建流失败）。
    const STANDARD_RATES: [u32; 8] = [22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000];
    let supported_rates: Vec<u32> = device
        .f32_rate_ranges()
        .map(|ranges| {
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

pub(crate) fn negotiate_sample_rate(device: &Device, requested: u32, device_default: u32) -> u32 {
    let ranges = device.f32_rate_ranges();
    negotiate_sample_rate_in(requested, device_default, ranges.as_deref())
}

pub(crate) fn negotiate_sample_rate_in(
    requested: u32,
    device_default: u32,
    ranges: Option<&[(u32, u32)]>,
) -> u32 {
    let Some(ranges) = ranges else {
        return requested;
    };
    if ranges
        .iter()
        .any(|&(min, max)| requested >= min && requested <= max)
    {
        requested
    } else {
        tracing::warn!(
            "Sample rate {requested} Hz not supported by output device, \
             falling back to device default {device_default} Hz"
        );
        device_default
    }
}

pub(crate) fn negotiate_buffer_size(
    requested: BufferSize,
    supported: Option<(u32, u32)>,
) -> BufferSize {
    let BufferSize::Fixed(n) = requested else {
        return requested;
    };
    let Some((min, max)) = supported else {
        return requested;
    };
    let clamped = n.clamp(min, max);
    if clamped != n {
        tracing::warn!("Buffer size {n} out of device range {min}..={max}, clamped to {clamped}");
    }
    BufferSize::Fixed(clamped)
}

pub(crate) fn select_output_device(device_name: Option<&str>) -> Result<Device, String> {
    audio_out::find_output_device(device_name)
}
