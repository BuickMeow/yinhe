//! RBJ cookbook biquad 系数。

/// RBJ cookbook biquad 系数（与 xsynth 的 biquad crate 完全一致）。
/// 返回 (b0, b1, b2, a1, a2)，用于 DirectForm1：
/// y = b0*x + b1*x1 + b2*x2 - a1*y1 - a2*y2
///
/// cutoff 先按 xsynth `sanitize_freq` clamp 到 [1, Nyquist-1]：
/// 通道级 CC74 的 FREQS 表在极端值时（如 127）会算出远超 Nyquist 的
/// 频率，未 clamp 的系数会让 DF1 数值不稳定产生自激振荡（啸叫）。
pub fn biquad_coeffs(
    filter_type: u32,
    cutoff: f32,
    resonance: f32,
    sample_rate: f32,
) -> (f32, f32, f32, f32, f32) {
    let nyquist = (sample_rate * 0.5).max(1.0);
    let cutoff = cutoff.clamp(1.0, (nyquist - 1.0).max(1.0));
    let omega = 2.0 * std::f32::consts::PI * cutoff / sample_rate;
    let q = if resonance > 0.0 {
        resonance
    } else {
        std::f32::consts::FRAC_1_SQRT_2
    };
    match filter_type {
        3 => {
            // SinglePoleLowPass
            let omega_t = (omega / 2.0).tan();
            let a0 = 1.0 + omega_t;
            let b0 = omega_t / a0;
            ((b0), (b0), 0.0, (omega_t - 1.0) / a0, 0.0)
        }
        1 => {
            // HighPass
            let omega_s = omega.sin();
            let omega_c = omega.cos();
            let alpha = omega_s / (2.0 * q);
            let b0 = (1.0 + omega_c) * 0.5;
            let a0 = 1.0 + alpha;
            (
                b0 / a0,
                -b0 * 2.0 / a0,
                b0 / a0,
                -2.0 * omega_c / a0,
                (1.0 - alpha) / a0,
            )
        }
        2 => {
            // BandPass
            let omega_s = omega.sin();
            let omega_c = omega.cos();
            let alpha = omega_s / (2.0 * q);
            let a0 = 1.0 + alpha;
            let div = 1.0 / a0;
            (
                omega_s / 2.0 * div,
                0.0,
                -omega_s / 2.0 * div,
                -2.0 * omega_c * div,
                (1.0 - alpha) * div,
            )
        }
        _ => {
            // LowPass
            let omega_s = omega.sin();
            let omega_c = omega.cos();
            let alpha = omega_s / (2.0 * q);
            let b0 = (1.0 - omega_c) * 0.5;
            let a0 = 1.0 + alpha;
            (
                b0 / a0,
                2.0 * b0 / a0,
                b0 / a0,
                -2.0 * omega_c / a0,
                (1.0 - alpha) / a0,
            )
        }
    }
}
