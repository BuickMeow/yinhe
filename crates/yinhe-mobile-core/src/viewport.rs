//! 画布逻辑尺寸（Flutter 控件逻辑像素）与通用可见轨过滤。
//!
//! 平台入口（Android/iOS/OHOS）通过 `set_logical_size` 上报，场景/手势/
//! 渲染共用同一份尺寸，避免各模块各自维护。

use std::sync::atomic::{AtomicU32, Ordering};

static LOGICAL_W_BITS: AtomicU32 = AtomicU32::new(0);
static LOGICAL_H_BITS: AtomicU32 = AtomicU32::new(0);

/// 画布逻辑尺寸（视图/手势坐标系；未收到 `nativeSetViewport` 前为 0）。
pub fn logical_size() -> (f32, f32) {
    (
        f32::from_bits(LOGICAL_W_BITS.load(Ordering::Relaxed)),
        f32::from_bits(LOGICAL_H_BITS.load(Ordering::Relaxed)),
    )
}

/// 平台侧上报画布逻辑尺寸。
pub fn set_logical_size(w: f32, h: f32) {
    LOGICAL_W_BITS.store(w.max(1.0).to_bits(), Ordering::Relaxed);
    LOGICAL_H_BITS.store(h.max(1.0).to_bits(), Ordering::Relaxed);
}

/// 单轨编辑模式下只显示当前轨；`None` 保持原显隐。
pub fn effective_visible(track_visible: &[bool], display_track: Option<usize>) -> Vec<bool> {
    match display_track {
        None => track_visible.to_vec(),
        Some(t) => (0..track_visible.len())
            .map(|i| i == t && track_visible[i])
            .collect(),
    }
}
