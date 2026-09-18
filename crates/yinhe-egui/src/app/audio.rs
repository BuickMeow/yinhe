use crate::app::App;
use rust_i18n::t;
use yinhe_editor_core::progress;

use crate::widgets::toast::kind::ToastKind;
use crate::widgets::toast::model::ProgressOutcome;
use crate::widgets::toast::state::ENGINE_PROGRESS_ID;

impl App {
    /// Notify the audio engine that the active document's model has changed (full
    /// rebuild: cc_events + audible_notes + chase). Use for automation edits,
    /// undo / redo, or any edit that may have touched automation lanes.
    ///
    /// 若 channel 激活状态翻转（首/末发声音符添加/删除，或 automation/PC 增删），
    /// 自动 teardown 引擎——`ChannelLayout` 创建后不可变，必须重建才能让新通道
    /// 被 dispatch。下一帧 `rebuild_audio_if_needed` 会用新 model 重新 spawn。
    pub(crate) fn notify_audio_model_changed(&mut self) {
        // 待激活文档加载中：音频已绑定新文档，编辑的是界面上的旧文档——
        // 不做翻转检测（否则会误 teardown 正在重建的新引擎）。
        if self.audio_state.pending_doc_activate.is_some() {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        if self.audio_state.handle.is_none() {
            return;
        }
        if self.channel_layout_flipped_for_doc(idx) {
            self.teardown_audio();
        } else if let Some(audio) = &self.audio_state.handle {
            // AM lane M/S 试听状态独立于模型，由 `set_am_ms` 动态掩码维护。
            audio.reload_notes(self.workspace.documents[idx].data.model.clone());
        }
    }

    /// Notify the audio engine that only notes have changed (no automation, no
    /// chase). Cheaper than `notify_audio_model_changed` — skips the expensive
    /// `flatten_automation_to_cc_events` rebuild and the linear chase scan.
    /// Use for pure note edits (move/drag/add/delete/paste/duplicate/transpose).
    ///
    /// 若 channel 激活状态翻转（首/末发声音符添加/删除），自动 teardown 引擎
    /// 并下一帧重建——同 `notify_audio_model_changed`。
    pub(crate) fn notify_notes_changed(&mut self) {
        // 同 notify_audio_model_changed：待激活加载中不做增量/翻转处理。
        if self.audio_state.pending_doc_activate.is_some() {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        if self.audio_state.handle.is_none() {
            return;
        }
        if self.channel_layout_flipped_for_doc(idx) {
            self.teardown_audio();
        } else if let Some(audio) = &self.audio_state.handle {
            audio.update_notes(self.workspace.documents[idx].data.model.clone());
        }
    }

    /// 检测 doc idx 的当前 model 是否用到了引擎 `ChannelLayout` 里未激活的通道。
    ///
    /// 返回 true 表示必须 teardown 重建：`ChannelLayout` 创建后不可变，
    /// 新通道（加音轨/改 port/channel）无法被 dispatch。
    /// 返回 false 表示引擎布局仍覆盖该 model（含"占用减少"：删音轨/移走通道），
    /// 可走便宜的 `UpdateNotes` / `ReloadNotes` 路径。
    ///
    /// 若引擎绑定的 doc 与 idx 不一致（tab 切换后的 1 帧延迟），也返回 true
    /// ——必须 teardown 重建（或由 `try_adopt_engine` 先过户）以绑定到新 doc。
    fn channel_layout_flipped_for_doc(&self, idx: usize) -> bool {
        let Some(layout) = &self.audio_state.last_channel_layout else {
            return true; // 引擎未 spawn 过，让 rebuild 处理
        };
        if self.audio_state.active_doc != Some(idx) {
            return true; // 绑定的 doc 不一致，必须重建
        }
        let model = &self.workspace.documents[idx].data.model;
        !layout.covers_model(model)
    }

    /// Resolve the merged SF configuration for the given document.
    ///
    /// Returns `(源通道, paths)` for every **激活**源通道：通道有工程内覆盖
    /// 时用覆盖列表，否则用全局音色库；都为空时回退内置 GeneralUser GS。
    pub(crate) fn resolve_sf_config(
        &self,
        doc: &yinhe_editor_core::document::Document,
    ) -> Vec<(u8, Vec<String>)> {
        let layout = yinhe_audio::channels_for_model(&doc.data.model);
        let global = &self.audio_settings.global_sf_config;
        let project = &doc.edit.project_sf;

        let global_paths: Vec<String> = global
            .entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.path.clone())
            .collect();
        let builtin = yinhe_editor_core::config::builtin_soundfont_path()
            .map(|p| p.to_string_lossy().to_string());

        let mut result: Vec<(u8, Vec<String>)> = Vec::new();
        // 注意：`num_channels().min(256) as u8` 在 256 时会截断为 0（全空）。
        let channel_count = layout.num_channels().min(256);
        for ch_raw in 0..channel_count {
            let ch = ch_raw as u8;
            if !layout.is_active(ch as usize) {
                continue;
            }
            // 通道覆盖优先；无覆盖用全局。
            let paths: Vec<String> = match project.overrides.iter().find(|(c, _)| *c == ch) {
                Some((_, entries)) => entries
                    .iter()
                    .filter(|e| e.enabled)
                    .map(|e| e.path.clone())
                    .collect(),
                None => global_paths.clone(),
            };
            // 都为空：内置 fallback（仍然没有就跳过该通道 = 静音）。
            let paths = if paths.is_empty() {
                builtin.iter().cloned().collect()
            } else {
                paths
            };
            if !paths.is_empty() {
                result.push((ch, paths));
            }
        }
        result
    }

    /// Rebuild the audio engine if the active document changed or audio was dropped.
    ///
    /// spawn 在后台线程执行（设备枚举 + AudioEngine::new + build_output_stream
    /// 都是慢操作，UI 线程同步执行会冻结几百 ms）；结果由每帧的
    /// `poll_audio_spawn` 收取并完成初始状态注入。
    pub(crate) fn rebuild_audio_if_needed(&mut self) {
        // 有新文档待激活（加载完成等音频就绪）：音频按"待激活文档"重建，
        // 而不是当前仍显示着的旧文档。
        let target = self
            .audio_state
            .pending_doc_activate
            .as_ref()
            .map(|p| p.idx)
            .or(self.workspace.active_doc);
        let idx = match target {
            Some(idx) => idx,
            None => return,
        };

        // 文档切换时清失败：仅切到与失败归属不同文档时才清，同文档保持不重试避免 30Hz 刷屏
        if let Some(err_doc) = self.audio_state.spawn_error_doc
            && err_doc != idx
        {
            self.audio_state.spawn_error = None;
            self.audio_state.spawn_error_doc = None;
        }

        // spawn 失败后不重试，等用户操作（切设备/改设置/切文档）再重试
        if self.audio_state.spawn_error.is_some() {
            return;
        }

        let needs_rebuild =
            self.audio_state.active_doc != Some(idx) || self.audio_state.handle.is_none();
        if !needs_rebuild {
            return;
        }

        // spawn 进行中：等结果（不在飞 spawn 上叠过户）。
        // 结果到达后若目标 doc 已变，下一帧再按过户/重建处理。
        if self.audio_state.spawn_rx.is_some() {
            return;
        }

        // 文档切换但引擎可复用（设置/布局/音色库都覆盖、机架无插件）：
        // 过户给新文档——不重开 cpal 流、不重建 GpuSynth、不重传采样。
        if self.try_adopt_engine(idx) {
            return;
        }

        progress::set_visible(&self.load_progress, true);
        progress::set_stage(&self.load_progress, 1, progress::StageStatus::Active);
        progress::set_stage_progress(&self.load_progress, 1, 0.0, "初始化音频引擎".into());

        // 引擎重建需要时间（spawn + 音色库/key map 加载，可能数秒）：建等待 toast。
        // 文件加载流程已有自己的 toast（同一 SharedProgress），避免重复建卡。
        if !self.file_loader.is_loading() {
            self.audio_state.engine_toast = Some(std::time::Instant::now());
            self.notifications.ensure_progress(
                ENGINE_PROGRESS_ID,
                ToastKind::Info,
                std::sync::Arc::new(crate::file_loader::LoadToastSource {
                    progress: self.load_progress.clone(),
                    cancel: None,
                    title: t!("toast.engine_switching").to_string(),
                }),
            );
        }

        // Drop old audio (stops cpal stream, frees engine)
        // 走 teardown_audio：渲染线程关机退回的 insert 处理器要交还机架
        // deactivate，不能直接丢句柄。
        self.teardown_audio();
        // 旧引擎的 audible_notes/cc_events 已释放：归还空闲页，切文档后 RSS 不累积。
        yinhe_memtrace::purge_free_pages();

        let doc = &self.workspace.documents[idx];
        let sr = self.audio_settings.sample_rate;
        let layout = yinhe_audio::channels_for_model(&doc.data.model);
        // spawn_cpal_audio 消费 layout，提前克隆一份作为快照，
        // 供后续 notify_notes_changed / notify_audio_model_changed 做 flip 检测。
        let layout_snapshot = layout.clone();
        let buffer_size = if self.audio_settings.buffer_size == 0 {
            cpal::BufferSize::Default
        } else {
            cpal::BufferSize::Fixed(self.audio_settings.buffer_size)
        };
        let device_name = self.audio_settings.output_device_name.clone();
        let synth_engine = self.audio_settings.synth_engine;
        let interpolation = self.audio_settings.interpolation;

        // 后台线程执行 spawn（设备枚举/线程池创建/建流全不在 UI 线程）。
        // 结果（+**发起时的设置快照**）经 mpsc 回传，UI 每帧 poll_audio_spawn
        // 收取；快照与当前设置不一致时丢弃（在飞期间用户改了设置）。
        let spawn_key = crate::app::audio_state::EngineSpawnKey::of(&self.audio_settings);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("audio-spawn".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    yinhe_audio::spawn_cpal_audio(
                        sr,
                        layout,
                        buffer_size,
                        device_name.as_deref(),
                        synth_engine,
                        interpolation,
                    )
                }));
                let result = match result {
                    Ok(r) => r.map(|audio| (audio, spawn_key)),
                    Err(payload) => Err(format!("Audio spawn panicked: {payload:?}")),
                };
                let _ = tx.send(result);
            })
            .expect("spawn audio thread");

        self.audio_state.spawn_for_doc = Some(idx);
        self.audio_state.spawn_rx = Some(rx);
        // spawn_restore_sample 由 switch_audio_device 设置，这里不覆盖
        // layout_snapshot 在 spawn 完成后由 poll_audio_spawn 设置
        self.audio_state.pending_layout = Some(layout_snapshot);
    }

    /// 每帧收取后台 spawn 结果：成功 → 注入初始状态（stage 1 Done）；
    /// 失败 → 记录错误。结果对应的 doc 已不是活动文档时丢弃（下一帧重新 spawn）。
    pub(crate) fn poll_audio_spawn(&mut self) {
        let Some(rx) = self.audio_state.spawn_rx.take() else {
            return;
        };
        let spawn_for = self.audio_state.spawn_for_doc.take();
        let restore = self.audio_state.spawn_restore_sample.take();
        let pending_layout = self.audio_state.pending_layout.take();
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // 还在跑：放回状态，下帧再查
                self.audio_state.spawn_rx = Some(rx);
                self.audio_state.spawn_for_doc = spawn_for;
                self.audio_state.spawn_restore_sample = restore;
                self.audio_state.pending_layout = pending_layout;
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("Audio spawn thread disconnected".to_string())
            }
        };

        match result {
            Ok((audio, spawn_key)) => {
                // 在飞期间设置已变（改插值/采样率/后端/设备等）：丢弃该结果，
                // 下一帧 rebuild_audio_if_needed 会按新设置重新 spawn。否则旧
                // 设置的引擎会被安装并被误记为最新（新设置永不生效）。
                if spawn_key != crate::app::audio_state::EngineSpawnKey::of(&self.audio_settings) {
                    drop(audio);
                    yinhe_memtrace::purge_free_pages();
                    return;
                }
                // 目标文档：与 rebuild_audio_if_needed 一致——有待激活文档时
                // 用它（加载完成但音频就绪前不切 active_doc），否则用当前文档。
                let target = self
                    .audio_state
                    .pending_doc_activate
                    .as_ref()
                    .map(|p| p.idx)
                    .or(self.workspace.active_doc);
                let Some(idx) = target else {
                    drop(audio);
                    return;
                };
                // 结果对应的是旧 doc（切换文档后旧 spawn 才完成）：丢弃，
                // 下一帧 rebuild_audio_if_needed 会用新 doc 重新 spawn。
                if spawn_for != Some(idx) {
                    drop(audio);
                    yinhe_memtrace::purge_free_pages();
                    return;
                }

                progress::set_stage(&self.load_progress, 1, progress::StageStatus::Done);

                let doc = &self.workspace.documents[idx];
                // 音色库完成计数基准（事件驱动进度：每完成一 port +1）
                let port_configs = self.resolve_sf_config(doc);
                self.audio_state.sf_total = port_configs.len();
                self.audio_state.sf_pending = true;

                // 音频素材：按实际采样率重新解码（若未解码过），并把已有 PCM
                // 重推给新引擎（引擎重建后 audio_sources 为空）。
                {
                    let model = self.workspace.documents[idx].data.model.clone();
                    let sr = audio.sample_rate;
                    self.audio_library.ensure_decoded(&model, sr);
                }
                let doc = &self.workspace.documents[idx];
                self.send_initial_audio_state(&audio, doc, &port_configs);

                // 设备切换：恢复播放位置，关对话框
                if let Some(sample) = restore {
                    audio
                        .handle
                        .send(yinhe_audio::AudioCommand::Seek { sample });
                    self.audio_state.device_switch_pending = false;
                    self.audio_state.device_switch_error = None;
                }

                self.audio_state.handle = Some(audio);
                self.audio_state.active_doc = Some(idx);
                self.audio_state.last_channel_layout = pending_layout;
                // 记录引擎创建快照：跨文档复用（try_adopt_engine）的判定依据。
                // 用发起时快照（已校验与当前设置一致），而非完成时刻的再读取。
                self.audio_state.engine_key = Some(spawn_key);
                self.audio_state.engine_sf_configs = port_configs.clone();
                // 成功后清失败状态，避免同文档下次 rebuild 被误拦
                self.audio_state.spawn_error = None;
                self.audio_state.spawn_error_doc = None;

                // 混音台：全量参数 + 各 insert 处理器激活补发（引擎是全新 spawn，
                // 机架里所有槽位此时都是未发送状态）。
                self.push_mixer_state_to_engine(idx);

                // 进度条保持可见：音色库异步加载的完成计数由
                // `poll_audio_progress` 驱动，全部完成才隐藏。
            }
            Err(e) => {
                tracing::error!("Failed to create audio: {}", e);
                self.audio_state.spawn_error = Some(e.clone());
                self.audio_state.spawn_error_doc = spawn_for;
                self.audio_state.device_switch_error = Some(e.clone());
                progress::set_visible(&self.load_progress, false);
                if self.audio_state.engine_toast.take().is_some() {
                    self.notifications.finish_progress(
                        ENGINE_PROGRESS_ID,
                        ProgressOutcome::Failed,
                        t!("toast.engine_failed").to_string(),
                        e,
                        None,
                    );
                }
            }
        }
    }

    /// 尝试把现有引擎"过户"给文档 `idx`——不 teardown + 重 spawn。
    ///
    /// 换文档的常规路径是"拆引擎 + 重开 cpal 流 + 重建 GpuSynth + 重传采样"，
    /// 即使音色库/设备/采样率毫无变化也要全量重来（数百 ms 到数秒）。满足
    /// 以下条件时直接把新文档的模型与通道状态推给现有引擎即可（毫秒级）：
    /// 1. 引擎活着且设置快照未变（采样率/缓冲/设备/GPU 开关/全局音色库）；
    /// 2. 引擎布局覆盖新文档的通道需求（`ChannelLayout::covers_model`）；
    /// 3. 新文档需要的音色库配置逐通道与引擎已加载的完全一致；
    /// 4. 新旧文档的机架都没有插件 insert / 插件乐器（引擎内部状态干净，
    ///    避免旧插件残留或需要替换语义）。
    ///
    /// 任一条件不满足返回 false，调用方继续走 teardown + spawn 老路。
    fn try_adopt_engine(&mut self, idx: usize) -> bool {
        if self.audio_state.handle.is_none() {
            return false;
        }
        let Some(engine_key) = self.audio_state.engine_key.as_ref() else {
            return false;
        };
        if *engine_key != crate::app::audio_state::EngineSpawnKey::of(&self.audio_settings) {
            return false;
        }
        let Some(layout) = self.audio_state.last_channel_layout.as_ref() else {
            return false;
        };
        let Some(old_idx) = self.audio_state.active_doc else {
            return false;
        };
        let model = self.workspace.documents[idx].data.model.clone();
        if !layout.covers_model(&model) {
            return false;
        }
        let port_configs = self.resolve_sf_config(&self.workspace.documents[idx]);
        if !sf_configs_cover(&self.audio_state.engine_sf_configs, &port_configs) {
            return false;
        }
        if !self.racks_plugin_free(old_idx) || !self.racks_plugin_free(idx) {
            return false;
        }

        // 过户：把新文档状态推给现有引擎（`send_initial_audio_state` 的增量
        // 版本——音色库已覆盖，跳过 SetSoundFonts/SetLayerCount）。
        let doc = &self.workspace.documents[idx];
        let Some(audio) = self.audio_state.handle.as_ref() else {
            return false;
        };
        audio.handle.send(yinhe_audio::AudioCommand::Stop);
        audio.handle.send(yinhe_audio::AudioCommand::LoadModel {
            model: model.clone(),
        });
        audio
            .handle
            .send(yinhe_audio::AudioCommand::SetMixerParams {
                params: Box::new(doc.mixer.clone()),
            });
        audio.handle.set_skip_tracks(doc.compute_skip_mask());
        audio.set_am_ms(std::sync::Arc::new(doc.edit.arr_am_ms.clone()));
        self.audio_library.push_all_to_engine(&audio.handle);
        let sample_rate = audio.sample_rate;
        self.audio_library.ensure_decoded(&model, sample_rate);

        self.audio_state.active_doc = Some(idx);
        self.audio_state.playback_anchor = None;
        self.audio_state.pending_playback = false;
        // 引擎与音色库都已就绪：不进入"等音色库加载"的进度卡阶段。
        self.audio_state.sf_total = 0;
        self.audio_state.sf_pending = false;
        progress::set_stage(&self.load_progress, 1, progress::StageStatus::Done);
        progress::set_stage(&self.load_progress, 2, progress::StageStatus::Done);
        true
    }

    /// 文档 idx 的两个机架是否都没有插件实例（引擎侧无本机架插件残留）。
    fn racks_plugin_free(&self, idx: usize) -> bool {
        self.mixer_racks.get(idx).is_none_or(|r| r.is_plugin_free())
            && self
                .instrument_racks
                .get(idx)
                .is_none_or(|r| r.is_plugin_free())
    }

    /// Send the initial state to a freshly spawned audio handle:
    /// automation density, model, layer count, soundfonts, mute/solo.
    ///
    /// 拆分自 `rebuild_audio_if_needed`，让 spawn 路径与初始状态注入解耦。
    fn send_initial_audio_state(
        &self,
        audio: &yinhe_audio::CpalAudioHandle,
        doc: &yinhe_editor_core::document::Document,
        port_configs: &[(u8, Vec<String>)],
    ) {
        // Apply automation density before LoadModel so the first prepare uses it
        audio
            .handle
            .send(yinhe_audio::AudioCommand::SetAutomationDensity {
                density: self.audio_settings.automation_event_density,
            });

        // Load MIDI
        audio.handle.send(yinhe_audio::AudioCommand::LoadModel {
            model: doc.data.model.clone(),
        });

        // Apply XSynth layer count
        let layers = if self.audio_settings.xsynth_layers == 0 {
            None
        } else {
            Some(self.audio_settings.xsynth_layers as usize)
        };
        audio
            .handle
            .send(yinhe_audio::AudioCommand::SetLayerCount { count: layers });

        // Load SoundFonts — resolved from global + project config
        //
        // 进度改事件驱动：每个 port 的 `LoadedSoundFont` 结果回传时计数器 +1，
        // UI 每帧轮询（见 `poll_audio_progress`）更新 stage 2 —— 不再发完命令
        // 就预填 100%（异步加载还没开始，是假进度）。
        progress::set_stage(&self.load_progress, 2, progress::StageStatus::Active);
        progress::set_stage_progress(
            &self.load_progress,
            2,
            0.0,
            format!("0/{}", port_configs.len()),
        );
        // 一条批量命令携带全部通道配置（命令通道容量小，逐通道发会被丢弃）。
        audio.handle.send(yinhe_audio::AudioCommand::SetSoundFonts {
            configs: Box::new(port_configs.to_vec()),
        });

        // Send initial mute/solo state (latest-wins 槽，必达)
        let skip = doc.compute_skip_mask();
        audio.handle.set_skip_tracks(skip);

        // AM lane M/S 试听旁通：引擎重建后必须重发，否则 UI 按钮点亮但旁通静默丢失。
        audio.set_am_ms(std::sync::Arc::new(doc.edit.arr_am_ms.clone()));

        // 音频素材：把素材库中已解码的 PCM 全部重推给新引擎。
        self.audio_library.push_all_to_engine(&audio.handle);
    }

    /// 每帧轮询音频素材解码：确保当前工程素材已提交解码；新结果推给引擎。
    ///
    /// 引擎重建后由 `send_initial_audio_state` 重推全部素材；这里只处理
    /// 增量（导入新素材 / 后台解码完成）。
    pub(crate) fn poll_audio_library(&mut self) {
        // 待激活加载中：素材解码/推送针对新文档（spawn 时统一处理），
        // 跳过旧文档的增量，避免把旧素材推给已绑定新文档的引擎。
        if self.audio_state.pending_doc_activate.is_some() {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let sample_rate = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.sample_rate)
            .unwrap_or(self.audio_settings.sample_rate);
        let model = self.workspace.documents[idx].data.model.clone();
        self.audio_library.ensure_decoded(&model, sample_rate);
        let arrived = self.audio_library.poll();
        if arrived.is_empty() {
            return;
        }
        if let Some(audio) = &self.audio_state.handle {
            for (uuid, decoded) in arrived {
                self.audio_library
                    .push_to_engine(&audio.handle, uuid, decoded);
            }
        }
    }

    /// 每帧轮询音色库加载进度：`sf_loaded_count() / sf_total` 驱动
    /// "加载音色库"stage 的真实进度，全部完成才标 Done 并隐藏进度条。
    /// 无音色库配置（total = 0）时立即完成。
    pub(crate) fn poll_audio_progress(&mut self) {
        if !self.audio_state.sf_pending {
            return;
        }
        let Some(audio) = &self.audio_state.handle else {
            // handle 已丢（如 spawn 失败/重建中）：放弃本轮等待
            self.audio_state.sf_pending = false;
            progress::set_visible(&self.load_progress, false);
            if self.audio_state.engine_toast.take().is_some() {
                self.notifications.finish_progress(
                    ENGINE_PROGRESS_ID,
                    ProgressOutcome::Aborted,
                    t!("toast.engine_failed").to_string(),
                    String::new(),
                    None,
                );
            }
            return;
        };
        let total = self.audio_state.sf_total;
        let loaded = audio.handle.sf_loaded_count();
        if total == 0 || loaded >= total {
            // 音色库已加载完，但音频可能还在做 GpuSynth 初始化/采样上传/管线
            // 预热（约数百 ms）：等 audio_ready 再宣布"加载完成"，保证用户
            // 看到完成时点播放能立即响应。
            if !audio.handle.audio_ready() {
                progress::set_stage_progress(
                    &self.load_progress,
                    2,
                    total as f32,
                    "初始化音频".to_string(),
                );
                progress::set_visible(&self.load_progress, true);
                return;
            }
            self.audio_state.sf_pending = false;
            progress::set_stage(&self.load_progress, 2, progress::StageStatus::Done);
            progress::set_visible(&self.load_progress, false);
            if let Some(t0) = self.audio_state.engine_toast.take() {
                self.notifications.finish_progress(
                    ENGINE_PROGRESS_ID,
                    ProgressOutcome::Completed,
                    t!("toast.engine_ready").to_string(),
                    String::new(),
                    Some(format!("{:.1}s", t0.elapsed().as_secs_f32())),
                );
            }
        } else {
            progress::set_stage_progress(
                &self.load_progress,
                2,
                loaded as f32 / total as f32,
                format!("{}/{}", loaded, total),
            );
            // 进度条还在跑：保持可见（set_stage_progress 会把 stage 置 Active）
            progress::set_visible(&self.load_progress, true);
        }
    }

    /// 切换音频输出设备（由"音频设备切换"对话框触发）。
    ///
    /// 流程：保存当前 sample_position → 更新设置 → drop 旧 handle →
    /// 后台 spawn（`rebuild_audio_if_needed`）→ `poll_audio_spawn` 完成后
    /// 发 Seek 恢复位置并关对话框。
    ///
    /// spawn 成功：清 `device_switch_pending`，对话框下帧消失。
    /// spawn 失败：保留 `device_switch_pending`，把错误塞进 `device_switch_error`，
    /// 对话框保持打开让用户重选。
    pub(crate) fn switch_audio_device(&mut self, device_name: String) {
        // 清除之前的 spawn 失败状态，允许用新设备重试
        self.audio_state.spawn_error = None;
        self.audio_state.spawn_error_doc = None;
        let saved_sample = self
            .audio_state
            .handle
            .as_ref()
            .map(|h| h.handle.sample_position())
            .unwrap_or(0);
        // 记录恢复位置：spawn 完成后由 poll_audio_spawn 发送 Seek。
        self.audio_state.spawn_restore_sample = Some(saved_sample);

        self.audio_settings.output_device_name = Some(device_name);
        self.audio_settings.save();

        // drop 旧 handle（teardown 会 join 渲染线程并把退回的 insert 处理器
        // 交还机架 deactivate），强制 rebuild
        self.teardown_audio();

        // 用新设备名重建（rebuild_audio_if_needed 会读 output_device_name，
        // 后台 spawn，结果由每帧 poll_audio_spawn 收取）
        self.rebuild_audio_if_needed();
    }

    /// Handle playback toggle/pause/stop and cursor sync.
    pub(crate) fn handle_playback(
        &mut self,
        toggle_play: bool,
        pause_return: bool,
        stop_play: bool,
    ) {
        let (idx, audio) = match (self.workspace.active_doc, &self.audio_state.handle) {
            (Some(idx), Some(audio)) => (idx, audio),
            _ => return,
        };

        let doc = &mut self.workspace.documents[idx];
        let handle = &audio.handle;

        if toggle_play {
            if handle.is_playing() {
                handle.send(yinhe_audio::AudioCommand::Pause);
                self.audio_state.pending_playback = false;
                let sample = handle.sample_position();
                let time = sample as f64 / audio.sample_rate as f64;
                doc.edit.cursor_tick = Some(doc.data.model.tempo_map.tick_at_time(time));
                doc.edit.playback.stop();
            } else {
                let tick = doc.edit.cursor_tick.unwrap_or(0.0);
                let cursor_sample = (doc.data.model.tempo_map.tick_to_seconds(tick as u64)
                    * audio.sample_rate as f64) as u64;
                let engine_sample = handle.sample_position();
                // If cursor is at the engine's position, just resume (no seek)
                if cursor_sample.abs_diff(engine_sample) < (audio.sample_rate as u64 / 10) {
                    handle.send(yinhe_audio::AudioCommand::Resume);
                } else {
                    handle.send(yinhe_audio::AudioCommand::Play {
                        from_sample: cursor_sample,
                    });
                }
                self.audio_state.pending_playback = true;
                self.audio_state.pending_playback_since = Some(std::time::Instant::now());
                doc.edit.playback.toggle_play(tick, &doc.data.model);
            }
        }
        if pause_return {
            handle.send(yinhe_audio::AudioCommand::Pause);
            self.audio_state.pending_playback = false;
            let sample = handle.sample_position();
            let time = sample as f64 / audio.sample_rate as f64;
            doc.edit.cursor_tick = Some(doc.data.model.tempo_map.tick_at_time(time));
            doc.edit.playback.stop();
        }
        if stop_play {
            handle.send(yinhe_audio::AudioCommand::Stop);
            self.audio_state.pending_playback = false;
            doc.edit.cursor_tick = Some(0.0);
            doc.edit.playback.stop();
        }

        // 光标推进交给 interpolate_playback_cursor 独占处理～
        // 这里千万不要每帧重置 playback_anchor，不然插值会被压成一帧、变得一卡一卡的喵！
    }

    /// Between audio callback updates, interpolate the cursor position
    /// using the last known anchor + elapsed wall-clock time.
    /// Call this every frame during playback for smooth cursor motion.
    pub(crate) fn interpolate_playback_cursor(&mut self) {
        let (idx, audio) = match (self.workspace.active_doc, &self.audio_state.handle) {
            (Some(idx), Some(audio)) => (idx, audio),
            _ => return,
        };
        let handle = &audio.handle;
        if !handle.is_playing() {
            self.audio_state.playback_anchor = None;
            // Clear pending flag once the audio thread has caught up
            if self.audio_state.pending_playback {
                // Audio thread hasn't processed the Play command yet.
                // Keep the flag set so request_repaint() keeps firing.
                // 诊断：1s 仍未确认 → 命令可能被通道丢弃（只报一次）。
                if let Some(t) = self.audio_state.pending_playback_since
                    && t.elapsed() > std::time::Duration::from_secs(1)
                {
                    tracing::warn!("[play] Play 命令 1s 未被音频线程确认（可能已被命令通道丢弃）");
                    self.audio_state.pending_playback_since = None;
                }
                return;
            }
            return;
        }
        // Audio is confirmed playing — clear the pending flag.
        self.audio_state.pending_playback = false;
        if let Some(t) = self.audio_state.pending_playback_since.take() {
            tracing::info!(
                "[play] UI 等待音频确认={:?} producer={} consumer={}",
                t.elapsed(),
                handle.producer_sample_position(),
                handle.sample_position()
            );
        }

        let sr = audio.sample_rate as f64;
        let doc = match self.workspace.documents.get_mut(idx) {
            Some(doc) => doc,
            None => return,
        };

        let now = std::time::Instant::now();
        let engine_sample = handle.sample_position();

        // Anchor = the last time the engine's atomic sample position actually
        // changed.  We only refresh it when engine_sample differs from the
        // anchored sample, so `elapsed` accumulates the true wall-clock time
        // since the audio callback last advanced the position.  Resetting the
        // anchor every frame would collapse `elapsed` to a single frame and
        // kill the interpolation (cursor would step once per callback).
        let interpolated_sample = match self.audio_state.playback_anchor {
            Some((anchor_sample, anchor_time)) if engine_sample == anchor_sample => {
                // Atomic unchanged — extrapolate from the anchor.
                let elapsed = now.saturating_duration_since(anchor_time);
                anchor_sample as f64 + elapsed.as_secs_f64() * sr
            }
            _ => {
                // Atomic advanced (or first frame) — re-anchor to it.
                self.audio_state.playback_anchor = Some((engine_sample, now));
                engine_sample as f64
            }
        };

        // 指示线不允许超过渲染器已产出的位置：Play/Seek 清空 ring 后，
        // 首个 chunk 渲染完成之前没有可听的音频（cpal 回调输出的是静音）。
        // 若此时继续按墙钟外推，指示线会在"准备播放"期间空跑，等声音真正
        // 响起时指示线已经领先一大截，开头听起来就像被吞掉了。
        // 钳制到 producer 位置后：音频没准备好，指示线就停在原地等。
        let produced = handle.producer_sample_position();
        let interpolated_sample = interpolated_sample.min(produced as f64);

        let time = interpolated_sample / sr;
        let tick = doc.data.model.tempo_map.tick_at_time(time);
        let end_tick = doc.data.model.tick_length as f64;
        if tick >= end_tick {
            handle.send(yinhe_audio::AudioCommand::Stop);
            doc.edit.cursor_tick = Some(0.0);
            doc.edit.playback.stop();
            self.audio_state.playback_anchor = None;
        } else {
            doc.edit.cursor_tick = Some(tick.max(0.0));
        }
    }

    /// 发送音符听觉预览请求（铅笔新建/拖拽、选框拖拽产生）。
    ///
    /// - 通道 = 目标音轨的全局通道；
    /// - 目标位置自动化状态 = `target_tick` 处的自动化（渲染器按 target_tick chase）；
    /// - 时长换算用目标位置的 Tempo（tempo_map 自动变速）；
    /// - 力度缺省时用该音轨最近修改力度（default_velocity）。
    ///
    /// 整组合并成一条 `PreviewNotes` 命令发送（替换旧组待触发音符；正在响的音符
    /// 继续响满 gate，松手时由 Stop 统一停止；且不占命令通道额度）。
    /// 时刻全在 tick 域（与编辑层一致），渲染线程内部转 sample。
    /// `exclusive`：PR 拖动/铅笔的替换式预览（同一时刻只响当前组）；
    /// MIDI 直通用 false（叠加式，和弦保持）。
    pub(crate) fn send_note_previews(
        &self,
        reqs: &[crate::piano_view::PreviewReq],
        exclusive: bool,
    ) {
        if reqs.is_empty() {
            return;
        }
        let (Some(idx), Some(audio)) =
            (self.workspace.active_doc, self.audio_state.handle.as_ref())
        else {
            return;
        };
        let doc = &self.workspace.documents[idx];
        let model = &doc.data.model;
        let mut notes: Vec<yinhe_audio::PreviewNoteParams> = Vec::new();
        // 该 MIDI 通道挂载了可用插件实例 → 插件预览（音色与播放一致）；
        // 否则 XSynth 预览。
        let mut plugin_notes: Vec<(u8, Vec<yinhe_audio::InstrumentPreviewNote>)> = Vec::new();
        let rack = self.instrument_racks.get(idx);
        let mut stop = false;
        for req in reqs {
            match req {
                crate::piano_view::PreviewReq::Note(p) => {
                    let Some(track) = model.tracks.get(p.track as usize) else {
                        continue;
                    };
                    let velocity = p
                        .velocity
                        .unwrap_or_else(|| doc.edit.default_velocity(p.track));
                    let plugin_channel = rack
                        .is_some_and(|r| r.has_instance(track.global_channel()))
                        .then_some(track.global_channel());
                    if let Some(ch) = plugin_channel {
                        let note = yinhe_audio::InstrumentPreviewNote {
                            midi_channel: track.channel,
                            key: p.key,
                            velocity,
                            target_tick: p.target_tick,
                            duration_ticks: p.duration_ticks,
                        };
                        match plugin_notes.iter_mut().find(|(c, _)| *c == ch) {
                            Some((_, list)) => list.push(note),
                            None => plugin_notes.push((ch, vec![note])),
                        }
                    } else {
                        notes.push(yinhe_audio::PreviewNoteParams {
                            channel: track.global_channel(),
                            key: p.key,
                            velocity,
                            target_tick: p.target_tick,
                            duration_ticks: p.duration_ticks,
                        });
                    }
                }
                crate::piano_view::PreviewReq::Stop => stop = true,
            }
        }
        // 同帧出现 Stop（Create 松手）时优先停止；否则整组替换。
        if stop {
            // Stop 走快速路径标志（渲染忙时命令通道满会丢命令，标志保证松手即停）。
            audio.handle.request_preview_stop();
            audio.handle.send(yinhe_audio::AudioCommand::PreviewStop);
            audio
                .handle
                .send(yinhe_audio::AudioCommand::PreviewInstrumentStop {
                    channel: None,
                    key: None,
                });
        } else if !notes.is_empty() || !plugin_notes.is_empty() {
            // 新预览组：清除待消费的 Stop 请求，避免被渲染器当作"松手后的堆积旧组"跳过。
            audio.handle.clear_preview_stop();
            if !notes.is_empty() {
                audio
                    .handle
                    .send(yinhe_audio::AudioCommand::PreviewNotes { notes, exclusive });
            }
            for (channel, notes) in plugin_notes {
                audio
                    .handle
                    .send(yinhe_audio::AudioCommand::PreviewInstrumentNotes {
                        channel,
                        notes,
                        exclusive,
                    });
            }
        }
    }

    /// MIDI 直通单键 NoteOff：只停该键的预览音（xsynth + 乐器插件通道），和弦保持。
    pub(crate) fn stop_preview_key(&self, key: u8) {
        let (Some(idx), Some(audio)) =
            (self.workspace.active_doc, self.audio_state.handle.as_ref())
        else {
            return;
        };
        audio
            .handle
            .send(yinhe_audio::AudioCommand::PreviewStopKey { key });
        let doc = &self.workspace.documents[idx];
        let Some(track) = doc
            .data
            .model
            .tracks
            .get(self.current_write_track() as usize)
        else {
            return;
        };
        if track.kind == yinhe_core::TrackKind::Audio {
            return;
        }
        let ch = track.global_channel();
        if self
            .instrument_racks
            .get(idx)
            .is_some_and(|r| r.has_instance(ch))
        {
            audio
                .handle
                .send(yinhe_audio::AudioCommand::PreviewInstrumentStop {
                    channel: Some(ch),
                    key: Some(key),
                });
        }
    }

    /// Tear down audio (e.g. on new project or settings change).
    ///
    /// `CpalAudioHandle::Drop` 会 join 渲染线程，而渲染线程退出时要释放 GPU
    /// 资源（device/buffer）并 `purge_free_pages`（jemalloc 归还 500MB 级采样
    /// 缓冲给 OS），可达数百 ms——同步做会把 UI 卡住（toast 停住、恢复后跳到
    /// 最新进度）。这里把 drop 扔到后台线程，渲染线程关机时退回的 insert
    /// 处理器由后台线程收齐后经 std mpsc 回传，`poll_insert_returns` 每帧取回。
    pub(crate) fn teardown_audio(&mut self) {
        // 先克隆接收端再 drop 句柄（关机退回的处理器经它取回）。
        let return_rx = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.clone_insert_return_rx());
        let bound_doc = self.audio_state.active_doc;
        if let Some(a) = self.audio_state.handle.take() {
            // 同步暂停输出流（立即静音）：drop 在后台线程做（join 渲染线程
            // 可能数百 ms），期间旧流仍会播完 ring 残余；新引擎可能已在
            // 另一线程 build+play → 双流重叠的设备二次配置"滋"。
            a.pause_stream();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let t = std::time::Instant::now();
                drop(a);
                let mut returned: Vec<Box<dyn yinhe_mixer::InsertProcessor>> = Vec::new();
                if let Some(rx) = return_rx {
                    while let Ok(mut batch) = rx.try_recv() {
                        returned.append(&mut batch);
                    }
                }
                let _ = tx.send(returned);
                tracing::info!("[audio] 后台 teardown（join + 回收）={:?}", t.elapsed());
            });
            if let Some(idx) = bound_doc {
                self.audio_state.pending_insert_returns = Some((rx, idx));
            }
        }
        self.audio_state.active_doc = None;
        self.audio_state.last_channel_layout = None;
        self.audio_state.engine_key = None;
        self.audio_state.engine_sf_configs.clear();
        self.audio_state.spawn_error = None;
        self.audio_state.spawn_error_doc = None;
    }

    /// 每帧收取后台 teardown 回传的 insert 处理器，交回机架 deactivate
    /// （CLAP deactivate 必须在 UI/管理线程做）。
    pub(crate) fn poll_insert_returns(&mut self) {
        let Some((rx, idx)) = self.audio_state.pending_insert_returns.as_ref() else {
            return;
        };
        let idx = *idx;
        let returned = match rx.try_recv() {
            Ok(returned) => returned,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        self.audio_state.pending_insert_returns = None;
        if !returned.is_empty() && idx < self.mixer_racks.len() {
            self.mixer_racks[idx].on_returns(returned);
        }
    }
}

/// `engine` 已加载的音色库配置是否覆盖 `needed`：`needed` 的每一项
/// 都能在 `engine` 里找到完全相同的 (源通道, paths)。
fn sf_configs_cover(engine: &[(u8, Vec<String>)], needed: &[(u8, Vec<String>)]) -> bool {
    needed
        .iter()
        .all(|(ch, paths)| engine.iter().any(|(c, p)| c == ch && p == paths))
}

#[cfg(test)]
mod adopt_tests {
    use super::sf_configs_cover;

    fn cfgs(items: &[(u8, &[&str])]) -> Vec<(u8, Vec<String>)> {
        items
            .iter()
            .map(|(ch, paths)| (*ch, paths.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    #[test]
    fn sf_configs_cover_subset_with_same_paths() {
        let engine = cfgs(&[(0, &["/a.sfz"]), (1, &["/a.sfz"])]);
        assert!(sf_configs_cover(&engine, &cfgs(&[(0, &["/a.sfz"])])));
        assert!(sf_configs_cover(&engine, &[]));
        assert!(sf_configs_cover(&engine, &engine));
    }

    #[test]
    fn sf_configs_cover_rejects_missing_or_different() {
        let engine = cfgs(&[(0, &["/a.sfz"])]);
        // 新文档用到的通道引擎没加载
        assert!(!sf_configs_cover(&engine, &cfgs(&[(1, &["/a.sfz"])])));
        // 同通道但路径不同（工程覆盖）
        assert!(!sf_configs_cover(&engine, &cfgs(&[(0, &["/b.sfz"])])));
        // 同通道路径追加
        assert!(!sf_configs_cover(
            &engine,
            &cfgs(&[(0, &["/a.sfz", "/b.sfz"])])
        ));
        // 引擎空，需求非空
        assert!(!sf_configs_cover(&cfgs(&[]), &cfgs(&[(0, &["/a.sfz"])])));
    }
}
