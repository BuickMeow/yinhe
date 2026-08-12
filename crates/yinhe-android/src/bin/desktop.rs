//! 桌面端调试入口：与安卓共用同一个 [`YinheApp`]，便于在 mac 上快速
//! 迭代 UI 与触摸逻辑（用鼠标/触控板模拟）。

fn main() {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    yinhe_android::run(options).unwrap();
}
