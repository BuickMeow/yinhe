//! worker 结果的应用（模型/音符/音色库/chase）：渲染线程的命令循环调用。

use std::sync::atomic::Ordering;
use std::time::Instant;

use crossbeam_channel::TryRecvError;

use crate::spawn::WorkerResult;

use super::{AudioRenderer, play_log};

impl AudioRenderer {
    pub(super) fn process_worker_results(&mut self) -> bool {
        let mut did_work = false;
        loop {
            match self.prepared_rx.try_recv() {
                Ok(WorkerResult::PreparedModel(prepared)) => {
                    let t_prepared = Instant::now();
                    self.state
                        .duration_samples
                        .store(prepared.duration_samples, Ordering::Relaxed);
                    // 锚定听音位置（非显式 reload 不移动播放位置）。
                    let anchor = self.consumer_position.load(Ordering::Acquire);
                    self.engine.apply_prepared_model(prepared, anchor);
                    // GPU 路径：模型变化 → 事件列表失效（run 循环统一重建）
                    #[cfg(feature = "gpu")]
                    self.engine.invalidate_gpu_events();
                    self.clear_buffered_audio(anchor);
                    self.state.initialized.store(true, Ordering::Release);
                    // 方案 B：apply_prepared_model 内部 seek_to 不再 chase，
                    // 这里发 PrepareChase 让 worker 异步算 channel state
                    self.request_chase(self.engine.current_tick());
                    // 模型已就绪且没有待加载音色库（无配置/已加载完）→ 音频就绪。
                    if self.gpu_sf_pending == 0 {
                        self.mark_audio_ready();
                    }
                    play_log(&format!("[play] 应用模型结果={:?}", t_prepared.elapsed()));
                    did_work = true;
                }
                Ok(WorkerResult::PreparedNotes {
                    model,
                    yin_model,
                    audible_delta,
                    duration_samples,
                }) => {
                    self.state
                        .duration_samples
                        .store(duration_samples, Ordering::Relaxed);
                    self.engine
                        .apply_notes_only(model, yin_model, audible_delta, duration_samples);
                    // GPU 路径：音符变化 → 事件列表失效（run 循环统一重建）
                    #[cfg(feature = "gpu")]
                    self.engine.invalidate_gpu_events();
                    // 注意：这里**不**清 ring。UpdateNotes 不 seek、不改 cc_events，
                    // 已渲染的 ring 内容是"过去时"音频（新音符只影响未来 dispatch），
                    // 清空会把正在播放的预览余音/当前音频丢掉 → 松手停顿。
                    // 只有 PreparedModel（cc_events 重建/seek）才需要清。
                    self.state.initialized.store(true, Ordering::Release);
                    did_work = true;
                }
                Ok(WorkerResult::ChaseResult {
                    states,
                    plugin_params,
                    generation,
                }) => {
                    let t_chase_apply = Instant::now();
                    // 丢弃过期结果：cc_events 已被新 PrepareModel 替换
                    if generation == self.engine.chase_generation {
                        // GPU 路径的 chase 应用已并入 apply_chase_result 内部
                        //（apply_gpu_chase），此处无需额外分支。
                        self.engine.apply_chase_result(&states, &plugin_params);
                        if self.play_timing.is_some() {
                            play_log(&format!(
                                "[play] chase 快照应用={:?}",
                                t_chase_apply.elapsed()
                            ));
                        }
                        did_work = true;
                    }
                }
                Ok(WorkerResult::LoadedSoundFont {
                    channel,
                    soundfonts,
                    paths,
                }) => {
                    // 音色库完成计数：UI 的"加载音色库"stage 进度 = 已完成通道数。
                    self.state.sf_loaded.fetch_add(1, Ordering::Relaxed);
                    // 预览引擎与主引擎共享同一音色（Arc，零拷贝）。
                    self.preview_engine
                        .set_channel_soundfonts(channel, soundfonts.clone());
                    let dense = self.engine.channel_layout.dense_for(channel as usize);
                    self.engine
                        .apply_loaded_soundfont_for_channel(channel, dense, soundfonts);
                    // GPU 路径：首次加载音色库时初始化 GpuSynth，后续通道逐个加载；
                    // 样本统一在最后一个通道完成时上传一次（避免逐通道全量重传）。
                    #[cfg(feature = "gpu")]
                    if self.gpu_engine()
                        && dense != u32::MAX
                        && (dense as usize) < yinhe_synth::MAX_CHANNELS
                    {
                        let sr = self.engine.sample_rate;
                        let gpu_paths: Vec<std::path::PathBuf> =
                            paths.iter().map(std::path::PathBuf::from).collect();
                        if self.engine.gpu_synth.is_none() {
                            let t_init = Instant::now();
                            match yinhe_synth::GpuSynth::new_default(sr) {
                                Ok(mut synth) => {
                                    let t_load = Instant::now();
                                    if let Err(e) = synth.load_dense_soundfonts(dense, &gpu_paths) {
                                        eprintln!("[gpu] Failed to load soundfonts: {e}");
                                    }
                                    let dt_load = t_load.elapsed();
                                    self.engine.gpu_synth = Some(synth);
                                    // 新后端实例：置位 dirty 后立即同步（构建事件表 + seek）。
                                    self.engine.invalidate_gpu_events();
                                    self.engine.sync_gpu_backend();
                                    eprintln!("[gpu] GpuSynth initialized (channel {channel})");
                                    play_log(&format!(
                                        "[play] GpuSynth 初始化：new={:?} 音色库解析={dt_load:?} 总={:?}",
                                        t_load.duration_since(t_init),
                                        t_init.elapsed()
                                    ));
                                }
                                Err(e) => {
                                    eprintln!("[gpu] Failed to init GpuSynth: {e}");
                                }
                            }
                        } else if let Some(synth) = self.engine.gpu_synth.as_mut()
                            && let Err(e) = synth.load_dense_soundfonts(dense, &gpu_paths)
                        {
                            eprintln!("[gpu] Failed to load channel {channel} soundfonts: {e}");
                        }
                        self.gpu_sf_pending = self.gpu_sf_pending.saturating_sub(1);
                        if self.gpu_sf_pending == 0
                            && let Some(synth) = self.engine.gpu_synth.as_mut()
                        {
                            let t_upload = Instant::now();
                            synth.finish_soundfont_load();
                            // 预热 GPU 缓冲（满 voice 容量 + 段长）：把首次分配
                            // 从播放阶段（第一个音符触发扩容）移到加载阶段。
                            #[cfg(feature = "gpu")]
                            synth.prewarm(super::GPU_RENDER_CHUNK_FRAMES as u32);
                            play_log(&format!(
                                "[play] GPU 采样上传完成：{:?}",
                                t_upload.elapsed()
                            ));
                            if self.gpu_sf_pending == 0 {
                                self.mark_audio_ready();
                            }
                        }
                    }
                    // 非 GPU feature 下 paths 不使用，显式标记避免 warning
                    #[cfg(not(feature = "gpu"))]
                    let _ = paths;
                    did_work = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        did_work
    }
}
