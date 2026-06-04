mod app;
mod arrange;
mod arrangement_view_ui;
mod document;
mod file_loader;
mod loading;
mod mode_bar;
mod piano_view;
mod playback;
mod quantize;
mod render_context;
mod time_format;
mod title_bar;
mod track_panel;
mod transport_bar;
mod view_interaction;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
                .parse_lossy("wgpu=warn,naga=warn"),
        )
        .init();

    let mut viewport = eframe::egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]);

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
