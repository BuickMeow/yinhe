use yinhe_memtrace::TaggedAlloc;

#[global_allocator]
static GLOBAL_ALLOC: TaggedAlloc = TaggedAlloc;

mod app;
mod arrange;
mod audio_settings;
mod config;
mod dialogs;
mod file_loader;
mod perf_probe;
mod piano_view;
mod progress;
mod render_context;
mod right_panel;
mod theme;
mod util;
mod view_interaction;
mod chrome;
mod widgets;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
                .parse_lossy("wgpu=warn,naga=warn"),
        )
        .init();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1400.0, 900.0])
        .with_transparent(true); // Avoid white flash before first frame

    let icon_data = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
        let icon = image::load_from_memory(include_bytes!("../../../assets/icon.png"))
            .expect("Failed to load window icon")
            .to_rgba8();
        let (icon_w, icon_h) = icon.dimensions();
        egui::IconData {
            rgba: icon.into_raw(),
            width: icon_w,
            height: icon_h,
        }
    });
    viewport = viewport.with_icon(icon_data);

    #[cfg(target_os = "macos")]
    {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }

    #[cfg(not(target_os = "macos"))]
    {
        viewport = viewport.with_decorations(false);
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Yinhe MIDI Editor",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
    .unwrap();
}
