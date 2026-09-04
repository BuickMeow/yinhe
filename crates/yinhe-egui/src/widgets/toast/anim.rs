use std::time::Instant;

use super::model::Toast;

pub(crate) fn fly_anim(toast: &Toast, stagger: f32) -> (f32, f32) {
    let now = Instant::now();
    // 飞入距离 = 卡片宽度 + 右侧间距 + 阴影，确保完全在屏幕外起步
    const DIST: f32 = 420.0;
    if let Some(since) = toast.leaving_since {
        let t = (now.duration_since(since).as_secs_f32() / 0.28).clamp(0.0, 1.0);
        let e = t * t * t;
        let x = e * DIST;
        return (x, 1.0);
    }
    let elapsed = now.duration_since(toast.created).as_secs_f32() - stagger;
    if elapsed < 0.0 {
        return (DIST, 1.0);
    }
    let t = (elapsed / 0.32).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - t).powi(3);
    let x = (1.0 - e) * DIST;
    (x, 1.0)
}

pub(crate) fn mul_alpha(c: egui::Color32, a: f32) -> egui::Color32 {
    let a = a.clamp(0.0, 1.0);
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * a) as u8)
}
