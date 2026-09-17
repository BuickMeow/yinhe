//! 命令处理：传输控制（Play/Seek/Stop）、掩码、参数与导出启动。
//!
//! 命令来源分两路（见 `renderer.run`）：可靠无界通道（传输命令，保序必达）
//! 与有界通道（普通命令，满则丢并告警）；latest-wins 槽兜底 M/S 类必达命令。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crossbeam_channel::TryRecvError;

use crate::spawn::{AmMsMap, AudioCommand, WorkerCmd};

use super::{AudioRenderer, merge_transport_batch, play_log};

impl AudioRenderer {
    pub(super) fn process_commands(&mut self) -> bool {
        let mut did_work = false;
        let mut pending_reload: Option<Arc<yinhe_core::YinModel>> = None;
        let mut pending_update_notes: Option<Arc<yinhe_core::YinModel>> = None;
        let mut pending_density_rebuild: bool = false;

        // latest-wins 槽：M/S 掩码必达（命令通道满合并时不丢最新值）。
        // 先取值、放锁，再应用（避免临时 guard 借用贯穿 &mut self 调用）。
        let pending_skip = self
            .pending_skip
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(skip) = pending_skip {
            self.apply_skip_tracks(skip);
            did_work = true;
        }
        let pending_am_ms = self
            .pending_am_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(am_ms) = pending_am_ms {
            self.apply_set_am_ms(am_ms);
            did_work = true;
        }

        // 可靠命令（独立无界通道）：优先、保序处理，永不丢。
        // 批内连续 Seek 合并为最后一个（绝对位置，中间值无需逐条同步）。
        let mut transport: Vec<AudioCommand> = Vec::new();
        while let Ok(cmd) = self.transport_rx.try_recv() {
            transport.push(cmd);
        }
        if !transport.is_empty() {
            for cmd in merge_transport_batch(transport) {
                did_work = true;
                self.apply_command(
                    cmd,
                    &mut pending_reload,
                    &mut pending_update_notes,
                    &mut pending_density_rebuild,
                );
            }
        }

        loop {
            match self.cmd_rx.try_recv() {
                Ok(cmd) => {
                    did_work = true;
                    self.apply_command(
                        cmd,
                        &mut pending_reload,
                        &mut pending_update_notes,
                        &mut pending_density_rebuild,
                    );
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return did_work,
            }
        }

        if let Some(model) = pending_reload {
            // 不提前清 ring / 不 seek：已渲染（旧模型）音频继续播到 PreparedModel
            // 应用时，由 apply_prepared_model(consumer) 无缝接新模型——位置不移动。
            let density = self.engine.automation_density;
            let _ = self.worker_tx.send(WorkerCmd::PrepareModel(model, density));
            did_work = true;
        } else if let Some(model) = pending_update_notes {
            // 只更新音符，不重建 cc_events，不 chase
            let _ = self.worker_tx.send(WorkerCmd::PrepareNotes(model));
            did_work = true;
        } else if pending_density_rebuild {
            // density 改变后用当前模型重建 cc_events
            if let Some(model) = self.engine.yin_model.clone() {
                let density = self.engine.automation_density;
                let _ = self.worker_tx.send(WorkerCmd::PrepareModel(model, density));
                did_work = true;
            }
        }

        did_work
    }

    /// 应用轨道 mute/solo 掩码（即时派）：diff 旧掩码，新 mute 的轨立即停发 NoteOff，
    /// 新 unmute 的轨立即重启跨点音符（CPU/GPU 统一语义），随后异步 chase 恢复 CC。
    /// 应用单条命令（传输/普通命令通道共用）。
    pub(super) fn apply_command(
        &mut self,
        cmd: AudioCommand,
        pending_reload: &mut Option<Arc<yinhe_core::YinModel>>,
        pending_update_notes: &mut Option<Arc<yinhe_core::YinModel>>,
        pending_density_rebuild: &mut bool,
    ) {
        match cmd {
            AudioCommand::LoadModel { model } => {
                // 新的加载流程开始：音频重置为未就绪（音色库/采样/管线
                // 全部就绪后由 mark_audio_ready 置回）。
                self.state.audio_ready.store(false, Ordering::Release);
                self.preview_engine.stop_all();
                self.engine.handle_command(AudioCommand::Pause);
                self.engine.handle_command(AudioCommand::Stop);
                // 首次加载：消费位置 == 前沿 == 0，锚定无差别。
                self.clear_buffered_audio(self.engine.sample_position());
                let density = self.engine.automation_density;
                let _ = self.worker_tx.send(WorkerCmd::PrepareModel(model, density));
            }
            AudioCommand::ReloadNotes { model } => {
                // 全量重建优先于只更新音符 —— 丢弃 pending UpdateNotes
                *pending_update_notes = None;
                *pending_reload = Some(model);
            }
            AudioCommand::UpdateNotes { model } => {
                // 只在没有 pending ReloadNotes 时记录（ReloadNotes 包含 audible_notes）
                if pending_reload.is_none() {
                    *pending_update_notes = Some(model);
                }
            }
            AudioCommand::SetSoundFonts { configs } => {
                // 一次性清 ring（音色切换锚定听音位置，位置不移动）；
                // 结果逐个应用时不再清，避免频繁打断输出。
                let anchor = self.consumer_position.load(Ordering::Acquire);
                self.clear_buffered_audio(anchor);
                #[cfg(feature = "gpu")]
                {
                    self.gpu_sf_pending = super::count_gpu_sf_pending(
                        &configs,
                        &self.engine.channel_layout,
                        self.gpu_engine(),
                    );
                }
                // yinhe 后端（GPU 或 CPU）都在 worker 预热 key map 解析缓存；
                // xsynth 后端不需要（自己的 GLOBAL_SF_CACHE 在 worker 里加载）。
                #[cfg(feature = "gpu")]
                let prefetch_keymaps = self.gpu_engine() || self.yinhe_cpu_engine();
                #[cfg(not(feature = "gpu"))]
                let prefetch_keymaps = false;
                for (channel, paths) in configs.iter() {
                    if paths.is_empty() {
                        continue;
                    }
                    let _ = self.worker_tx.send(WorkerCmd::LoadSoundFont {
                        channel: *channel,
                        paths: paths.clone(),
                        prefetch_keymaps,
                    });
                }
            }
            AudioCommand::Play { from_sample } => {
                if self.export.is_some() {
                    // 导出中忽略播放控制（取消用导出卡的停止按钮）。
                } else if self.engine.model_loaded() {
                    let t0 = Instant::now();
                    self.preview_engine.stop_all();
                    self.engine
                        .handle_command(AudioCommand::Play { from_sample });
                    let dt_engine = t0.elapsed();
                    // 显式 seek：ring 清空锚定引擎当前（=seek 后）位置。
                    // GPU 后端的事件重建/seek 由 run 循环的 sync_gpu_backend 统一消费。
                    let t2 = Instant::now();
                    self.clear_buffered_audio(self.engine.sample_position());
                    let dt_clear = t2.elapsed();
                    // 方案 B：seek 后异步 chase（current_tick 已由 seek 更新）
                    let t3 = Instant::now();
                    self.request_chase(self.engine.current_tick());
                    let dt_chase = t3.elapsed();
                    play_log(&format!(
                        "[play] engine.seek={dt_engine:?} clear_ring={dt_clear:?} chase_req={dt_chase:?} 命令处理总计={:?}",
                        t0.elapsed()
                    ));
                    self.play_timing = Some((t0, from_sample));
                } else {
                    self.engine.set_pending_play(from_sample);
                    play_log("[play] 模型未就绪，Play 挂起等待模型/音色库加载");
                    self.play_timing = Some((Instant::now(), from_sample));
                }
            }
            AudioCommand::Seek { sample } => {
                if self.export.is_some() {
                    // 导出中忽略 seek。
                } else {
                    self.preview_engine.stop_all();
                    self.engine.handle_command(AudioCommand::Seek { sample });
                    // 显式 seek：ring 清空锚定引擎当前（=seek 后）位置。
                    self.clear_buffered_audio(self.engine.sample_position());
                    // 方案 B：seek 后异步 chase（current_tick 已由 seek 更新）
                    self.request_chase(self.engine.current_tick());
                }
            }
            AudioCommand::Stop => {
                if self.export.is_some() {
                    // 导出中忽略停止（取消用导出卡的停止按钮）。
                } else {
                    self.preview_engine.stop_all();
                    self.engine.handle_command(AudioCommand::Stop);
                    // Stop = 显式 seek 到 0。
                    self.clear_buffered_audio(self.engine.sample_position());
                    // 方案 B：Stop 也 seek 到 0，需要 chase 恢复初始 channel state
                    self.request_chase(0);
                }
            }
            AudioCommand::SetAutomationDensity { density } => {
                self.engine.automation_density = density.max(1);
                // 若已加载模型，触发 worker 重建 cc_events
                if self.engine.yin_model.is_some() {
                    *pending_density_rebuild = true;
                }
            }
            AudioCommand::SkipTracks { skip } => {
                self.apply_skip_tracks(skip);
            }
            AudioCommand::SetAmMs { am_ms } => {
                self.apply_set_am_ms(am_ms);
            }
            AudioCommand::PreviewNotes { notes, exclusive } => {
                // 用户已松手（Stop 请求尚未消费）：跳过堆积的旧预览组，
                // 否则松手后还会触发一组在响。
                if self.preview_stop_flag.load(Ordering::Acquire) {
                    return;
                }
                // 按 channel 分组、组内按 target_tick 升序，增量 chase：
                // 每个通道只扫一遍 cc_events，避免整组预览反复全量扫描。
                let cc_events = self.engine.cc_events.clone();
                let mut groups: Vec<(u32, Vec<&crate::spawn::PreviewNoteParams>)> = Vec::new();
                for n in &notes {
                    let ch = n.channel as u32;
                    if let Some((_, g)) = groups.iter_mut().find(|(c, _)| *c == ch) {
                        g.push(n);
                    } else {
                        groups.push((ch, vec![n]));
                    }
                }
                for (_, g) in &mut groups {
                    g.sort_by_key(|n| n.target_tick);
                }
                let mut inputs: Vec<crate::preview_engine::PreviewNoteIn> =
                    Vec::with_capacity(notes.len());
                for (ch, g) in groups {
                    let targets: Vec<u32> = g.iter().map(|n| n.target_tick).collect();
                    let states =
                        crate::preview_engine::chase_channel_states(&cc_events, ch, &targets);
                    for (n, state) in g.iter().zip(states.iter()) {
                        // 预览引擎内部时钟是渲染帧（sample 域）：tick 只用于
                        // chase 目标比较，这里把相对时值差/时长转回 sample。
                        let target = self.engine.tick_to_sample(n.target_tick);
                        let duration = if n.duration_ticks > 0 {
                            let end = self
                                .engine
                                .tick_to_sample(n.target_tick.saturating_add(n.duration_ticks));
                            Some(end.saturating_sub(target))
                        } else {
                            None
                        };
                        inputs.push(crate::preview_engine::PreviewNoteIn {
                            channel: n.channel,
                            key: n.key,
                            velocity: n.velocity,
                            duration,
                            state: *state,
                            target_sample: target,
                        });
                    }
                }
                // 提交预览组：组内按目标位置相对时值错开触发。
                self.preview_engine.preview_notes(inputs, exclusive);
            }
            AudioCommand::PreviewStop => {
                self.preview_engine.stop_all();
            }
            // MIDI 直通单键停止：只停松开的键（和弦保持）。
            AudioCommand::PreviewStopKey { key } => {
                self.preview_engine.stop_key(key);
            }
            AudioCommand::SyncBusConfig { buses, sends } => {
                self.engine
                    .handle_command(AudioCommand::SyncBusConfig { buses, sends });
                self.sync_bus_meter_readings();
            }
            AudioCommand::ExportStart {
                path,
                bit_depth,
                layer_count,
                restore_layer_count,
                progress,
                cancel,
                pause,
            } => {
                self.start_export(
                    path,
                    bit_depth,
                    layer_count,
                    restore_layer_count,
                    progress,
                    cancel,
                    pause,
                );
            }
            other => self.engine.handle_command(other),
        }
    }

    pub(super) fn apply_skip_tracks(&mut self, skip: Vec<bool>) {
        let old = self.engine.skip_track.clone();
        self.engine.apply_skip_mask(&old, &skip);
        // mute/solo 状态变了：旧 skip mask 的异步 chase 结果必须作废
        //（递增 generation），否则快速连续切换时旧结果可能晚到并
        // 覆盖新状态——GPU 路径的通道状态依赖 chase 恢复，影响更大。
        self.engine.chase_generation = self.engine.chase_generation.wrapping_add(1);
        // GPU 路径：事件列表按新 skip mask 重建（mute/solo 即时生效，
        // 不再需要重启引擎）；位置不移动（与 CPU 路径第 4 层语义一致）。
        // 事件重建 + seek 由 run 循环消费 dirty 标志时统一执行。
        #[cfg(feature = "gpu")]
        if self.engine.gpu_synth.is_some() && self.engine.model_loaded() {
            self.engine.invalidate_gpu_events();
            let anchor = self.consumer_position.load(Ordering::Acquire);
            self.clear_buffered_audio(anchor);
        }
        // mute 状态变了，chase 结果需要更新：unmute 的轨道的 CC 需要恢复，
        // mute 的轨道的 CC 不再参与。
        if self.engine.model_loaded() {
            self.request_chase(self.engine.current_tick());
        }
    }

    /// 应用 AM lane M/S 试听旁通集：只换动态掩码 + 异步 chase，
    /// 不重建模型、不 seek（与 SkipTracks 同机制）。
    pub(super) fn apply_set_am_ms(&mut self, am_ms: Arc<AmMsMap>) {
        self.engine.am_ms = am_ms;
        if let Some(model) = self.engine.yin_model.clone() {
            self.engine.am_lane_skip =
                crate::audio_model::build_am_lane_skip(&model, &self.engine.am_ms);
        }
        // 掩码变了：旧 chase 结果作废，重算 channel state。
        self.engine.chase_generation = self.engine.chase_generation.wrapping_add(1);
        if self.engine.model_loaded() {
            self.request_chase(self.engine.current_tick());
        }
    }

    /// 方案 B：发 `PrepareChase` 给 worker 线程异步计算 256 通道状态快照。
    /// worker 完成后回传 `ChaseResult`，`process_worker_results` 应用。
    /// `chase_generation` 用于丢弃过期结果（模型被 PrepareModel 替换后）。
    pub(super) fn request_chase(&self, target_tick: u32) {
        let Some(model) = self.engine.yin_model.clone() else {
            return;
        };
        let generation = self.engine.chase_generation;
        let skip_mask = self.engine.skip_track.clone();
        let am_ms = self.engine.am_ms.clone();
        let _ = self.worker_tx.send(WorkerCmd::PrepareChase {
            model,
            target_tick,
            generation,
            skip_mask,
            am_ms,
        });
    }
}
