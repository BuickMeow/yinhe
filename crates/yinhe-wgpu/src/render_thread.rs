//! Independent render thread for GPU rendering, decoupled from the UI thread.
//!
//! The render thread owns an `InstanceRenderer` and renders into the
//! offscreen texture managed by `RenderContext`. The UI thread sends
//! `RenderJob`s via a channel; the render thread uploads, draws, and
//! submits GPU commands asynchronously. The UI thread then displays
//! the texture via `RenderContext::paint_texture_only()`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::renderer::InstanceRenderer;
use crate::vertex::Uniforms;

/// A render job sent from the UI thread to the render thread.
///
/// Contains pre-built instance data and uniforms so the render thread
/// only needs to do GPU upload + draw + submit — no CPU-heavy work.
pub struct RenderJob {
    pub width: u32,
    pub height: u32,
    pub uniforms: Uniforms,
    pub track_colors: crate::vertex::TrackColorsUniform,
    pub selection: crate::vertex::SelectionUniform,
    pub decor_layers: Vec<DecorLayerData>,
    pub note_layers: Vec<NoteLayerData>,
}

/// Pre-built decor layer data (32B DrawInstance).
pub struct DecorLayerData {
    pub instances: Vec<crate::vertex::DrawInstance>,
    pub cache_key: u64,
}

/// Pre-built note layer data (16B NoteInstance).
pub struct NoteLayerData {
    pub instances: Vec<crate::vertex::NoteInstance>,
    pub cache_key: u64,
    /// If true, always upload (ignore cache).
    pub force: bool,
}

/// Shared state between the render thread and the UI thread.
///
/// The render thread renders into `target_view`; the UI thread
/// displays it via `RenderContext::paint_texture_only()`.
struct SharedState {
    target_view: wgpu::TextureView,
    width: u32,
    height: u32,
}

/// Handle to the render thread from the UI side.
pub struct RenderThreadHandle {
    job_tx: std::sync::mpsc::Sender<RenderJob>,
    running: Arc<AtomicBool>,
    shared: Arc<Mutex<SharedState>>,
}

impl RenderThreadHandle {
    /// Spawn the render thread.
    ///
    /// `device`/`queue` are cloned from the shared `RenderState`.
    /// `initial_view` is the offscreen texture view from `RenderContext`
    /// that the render thread will draw into.
    pub fn spawn(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        initial_view: wgpu::TextureView,
        initial_width: u32,
        initial_height: u32,
    ) -> Self {
        let (job_tx, job_rx) = std::sync::mpsc::channel();

        let running = Arc::new(AtomicBool::new(true));

        let shared = Arc::new(Mutex::new(SharedState {
            target_view: initial_view,
            width: initial_width,
            height: initial_height,
        }));

        let running_clone = Arc::clone(&running);
        let shared_clone = Arc::clone(&shared);

        let mut renderer = InstanceRenderer::new(device.clone(), queue.clone(), format);

        std::thread::Builder::new()
            .name("yinhe-render".into())
            .stack_size(32 * 1024 * 1024) // 32 MB — TrackColorsUniform 等大结构压栈需要
            .spawn(move || {
                tracing::info!("Render thread started");
                loop {
                    if !running_clone.load(Ordering::Relaxed) {
                        break;
                    }

                    // Drain all pending jobs, keep only the latest
                    let mut latest_job = None;
                    while let Ok(job) = job_rx.try_recv() {
                        latest_job = Some(job);
                    }

                    let job: RenderJob = match latest_job {
                        Some(j) => j,
                        None => {
                            // Block until a new job arrives (avoids busy-waiting)
                            match job_rx.recv() {
                                Ok(j) => j,
                                Err(_) => break, // Channel closed
                            }
                        }
                    };

                    let state = shared_clone.lock().unwrap();
                    let target_view = state.target_view.clone();
                    let width = state.width;
                    let height = state.height;
                    drop(state); // Release lock before GPU work

                    // Upload uniforms
                    renderer.upload_uniforms(job.uniforms);
                    renderer.upload_track_colors(&job.track_colors);
                    renderer.upload_selection(&job.selection);

                    // Ensure enough layers
                    let total_layers = job.decor_layers.len() + job.note_layers.len();
                    renderer.ensure_layers(total_layers);

                    // Upload decor layers
                    let mut layer_idx = 0;
                    for dl in &job.decor_layers {
                        let cache_key = dl.cache_key;
                        let instances = &dl.instances;
                        renderer.upload_layer(layer_idx, cache_key, |out| {
                            out.extend_from_slice(instances);
                        });
                        layer_idx += 1;
                    }

                    // Upload note layers (cache_key=0 means force)
                    for nl in &job.note_layers {
                        let cache_key = nl.cache_key;
                        let instances = &nl.instances;
                        renderer.upload_note_layer(layer_idx, if nl.force { 0 } else { cache_key }, |out| {
                            out.extend_from_slice(instances);
                        });
                        layer_idx += 1;
                    }

                    // Draw + submit
                    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("render_thread_frame"),
                    });
                    renderer.draw(&mut encoder, &target_view, width, height);
                    queue.submit(std::iter::once(encoder.finish()));
                }
                tracing::info!("Render thread stopped");
            })
            .expect("failed to spawn render thread");

        Self {
            job_tx,
            running,
            shared,
        }
    }

    /// Send a render job to the render thread.
    pub fn send_job(&self, job: RenderJob) {
        let _ = self.job_tx.send(job);
    }

    /// Update the target texture view (called by `RenderContext` on resize).
    pub fn update_target(&self, view: wgpu::TextureView, width: u32, height: u32) {
        let mut state = self.shared.lock().unwrap();
        state.target_view = view;
        state.width = width;
        state.height = height;
    }

    /// Shut down the render thread.
    pub fn shutdown(&self) {
        self.running.store(false, Ordering::Release);
    }
}

impl Drop for RenderThreadHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}
