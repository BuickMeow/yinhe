use super::*;

impl App {
    /// 活跃文档的机架（不存在则补默认——机架与 documents 平行，正常路径必然同长）。
    pub(crate) fn mixer_rack_mut(&mut self, idx: usize) -> &mut MixerRack {
        if idx >= self.mixer_racks.len() {
            self.mixer_racks.resize_with(idx + 1, MixerRack::default);
        }
        &mut self.mixer_racks[idx]
    }

    /// 记录混音台编辑的 undo 起点（若尚无暂存）。
    ///
    /// 高频路径（推子/旋钮拖动）每帧调用；只在首次变更时 clone 一份
    /// `MixerParams` 起点快照，指针空闲后由
    /// [`App::commit_pending_mixer_undo`] 合并提交。
    pub(crate) fn note_mixer_edit(&mut self, label: &str) {
        if self.mixer_undo_pending.is_none()
            && let Some(idx) = self.workspace.active_doc
        {
            let doc = &self.workspace.documents[idx];
            self.mixer_undo_pending =
                Some((doc.doc_id, Box::new(doc.mixer.clone()), label.to_string()));
        }
    }

    /// 指针空闲（未按住任何鼠标键）时，把暂存的混音台编辑提交为一条 undo。
    /// 无实际变化（如点了 M/S 又点回来）时不产生 undo。
    pub(crate) fn commit_pending_mixer_undo(&mut self, ctx: &egui::Context) {
        let Some((doc_id, before, label)) = self.mixer_undo_pending.take() else {
            return;
        };
        if ctx.input(|i| i.pointer.any_down()) {
            self.mixer_undo_pending = Some((doc_id, before, label));
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        // 拖动期间用户切换过文档（极少数场景）：暂存与当前文档不符则丢弃。
        if doc.doc_id != doc_id {
            return;
        }
        if *before == doc.mixer {
            return;
        }
        let after = Box::new(doc.mixer.clone());
        let snapshot = doc.capture_snapshot();
        doc.push_undo(
            yinhe_editor_core::history::UndoAction::MixerParams { before, after },
            &label,
            snapshot,
        );
        doc.mark_mixer_dirty();
    }

    /// 更新任意混音节点的 strip：写持久化层 + 推引擎（高频路径）。
    pub(crate) fn apply_node_strip(
        &mut self,
        idx: usize,
        node: yinhe_types::ChannelId,
        params: StripParams,
    ) {
        let audibility = params.audibility();
        let mut skip = None;
        {
            let doc = &mut self.workspace.documents[idx];
            if doc.mixer.node_strip(node).audibility() != audibility {
                // 单按钮三态：Channel solo 会清空 Track solo，需要同步 skip mask。
                doc.set_channel_audibility(node, audibility);
                skip = Some(doc.compute_skip_mask());
            }
            doc.mixer_mut().set_node_strip(node, params);
        }
        if let Some(audio) = &self.audio_state.handle {
            audio
                .handle
                .send(yinhe_audio::AudioCommand::SetNodeStrip { node, params });
            if let Some(skip) = skip {
                audio.handle.set_skip_tracks(skip);
            }
        }
    }

    pub(crate) fn apply_master(&mut self, idx: usize, params: MasterParams) {
        let master = &mut self.workspace.documents[idx].mixer_mut().master.strip;
        master.gain = params.gain;
        master.pan = params.pan;
        if let Some(audio) = &self.audio_state.handle {
            audio
                .handle
                .send(yinhe_audio::AudioCommand::SetMasterParams { params });
        }
    }

    /// 工程加载后：按 MixerParams 的 InsertRef 重建机架。
    /// 实例只加载不激活——引擎此时尚未重建，spawn 完成后由
    /// `push_mixer_state_to_engine` → `ensure_all_sent` 统一激活补发。
    pub(crate) fn restore_mixer_rack(&mut self, idx: usize) {
        self.migrate_builtin_inserts(idx);
        let mixer = self.workspace.documents[idx].mixer.clone();
        let mut rack = MixerRack::default();
        for node in mixer.nodes() {
            for r in &node.inserts {
                let _ = rack.load_plugin(
                    node.id,
                    r.format,
                    &r.plugin_path,
                    &r.plugin_id,
                    &r.name,
                    r.state.as_deref(),
                    r.bypassed,
                );
            }
        }
        for r in &mixer.master.inserts {
            let _ = rack.load_plugin(
                InsertTarget::Master,
                r.format,
                &r.plugin_path,
                &r.plugin_id,
                &r.name,
                r.state.as_deref(),
                r.bypassed,
            );
        }
        if idx >= self.mixer_racks.len() {
            self.mixer_racks.resize_with(idx + 1, MixerRack::default);
        }
        self.mixer_racks[idx] = rack;
    }

    /// 迁移：移除旧工程残留的内置效果器（ChannelGain/Pan/Filter）insert。
    /// 通道处理已内置到音源的通道处理段（yinhe-audio `ChannelDspChain`），
    /// 保留会双重处理 CC7/10/11/71/74；确有移除时标脏以便保存清理结果。
    fn migrate_builtin_inserts(&mut self, idx: usize) {
        let doc = &mut self.workspace.documents[idx];
        if strip_builtin_inserts(&mut doc.mixer) {
            doc.mark_mixer_dirty();
        }
    }

    /// 工程加载后：按 MixerParams.instruments 重建乐器机架。
    /// 与 restore_mixer_rack 同理——只加载不激活，引擎重建后由
    /// push_mixer_state_to_engine → ensure_all_sent 统一激活补发。
    pub(crate) fn restore_instrument_rack(&mut self, idx: usize) {
        let mixer = self.workspace.documents[idx].mixer.clone();
        let mut rack = InstrumentRack::default();
        let doc = &self.workspace.documents[idx];
        for (ch, rack_params) in mixer.racks.iter().enumerate() {
            let hosts = doc.instrument_hosts(ch as u8);
            for r in &rack_params.instruments {
                for lc in &hosts {
                    let _ = rack.add(
                        ch as u8,
                        *lc,
                        r.uid,
                        r.format,
                        &r.plugin_path,
                        &r.plugin_id,
                        &r.name,
                        r.state.as_deref(),
                    );
                }
            }
        }
        if idx >= self.instrument_racks.len() {
            self.instrument_racks
                .resize_with(idx + 1, InstrumentRack::default);
        }
        self.instrument_racks[idx] = rack;
    }

    /// 同步每轨 LC 的机架链：删除已失效的，补齐新增的。
    pub(crate) fn sync_track_lc_racks(&mut self, idx: usize) {
        let Some(doc) = self.workspace.documents.get(idx) else {
            return;
        };
        let mixer = doc.mixer.clone();
        let handle = self.audio_state.handle.as_ref().map(|a| &a.handle);
        if idx < self.mixer_racks.len() {
            let valid_nodes: std::collections::HashSet<yinhe_types::ChannelId> = mixer
                .nodes()
                .map(|node| node.id)
                .chain(std::iter::once(yinhe_types::ChannelId::Master))
                .collect();
            self.mixer_racks[idx].retain_channels(&valid_nodes, handle);
            self.mixer_racks[idx].ensure_track_lc_chains(&mixer);
        }
    }

    /// 引擎 spawn 完成后：全量同步混音台（参数 + 各 insert 处理器补发 + 乐器安装）。
    pub(crate) fn push_mixer_state_to_engine(&mut self, idx: usize) {
        let Self {
            audio_state,
            workspace,
            mixer_racks,
            instrument_racks,
            ..
        } = self;
        let Some(audio) = audio_state.handle.as_ref() else {
            return;
        };
        // 失效冻结保留在工程里（UI 可显示/重新冻结），但推给引擎的副本剔除，
        // 播放回退到实时输出。
        let mut params = workspace.documents[idx].mixer.clone();
        params.strip_stale_freezes(workspace.documents[idx].data.revision);
        audio
            .handle
            .send(yinhe_audio::AudioCommand::SetMixerParams {
                params: Box::new(params),
            });
        if idx >= mixer_racks.len() {
            mixer_racks.resize_with(idx + 1, MixerRack::default);
        }
        mixer_racks[idx].ensure_all_sent(&audio.handle, audio.sample_rate);
        if idx >= instrument_racks.len() {
            instrument_racks.resize_with(idx + 1, InstrumentRack::default);
        }
        instrument_racks[idx].ensure_all_sent(&audio.handle, audio.sample_rate);
    }

    /// 每帧：回收渲染线程退回的 insert 处理器 + 轮询插件反向请求。
    /// restart/移除退回的槽位在同一帧由 ensure_all_sent 补发（幂等，只补
    /// sent=false 的槽位）。
    pub(crate) fn poll_mixer_plugins(&mut self) {
        let Self {
            audio_state,
            mixer_racks,
            instrument_racks,
            ..
        } = self;
        let Some(audio) = audio_state.handle.as_ref() else {
            return;
        };
        let returned = audio.handle.drain_insert_returns();
        let instrument_returned = audio.handle.drain_instrument_returns();
        let Some(idx) = audio_state.active_doc else {
            if !returned.is_empty() {
                tracing::warn!(
                    "引擎退回 {} 个 insert 处理器，但无绑定文档可回收",
                    returned.len()
                );
            }
            if !instrument_returned.is_empty() {
                tracing::warn!(
                    "引擎退回 {} 个乐器处理器，但无绑定文档可回收",
                    instrument_returned.len()
                );
            }
            return;
        };
        if idx >= mixer_racks.len() {
            mixer_racks.resize_with(idx + 1, MixerRack::default);
        }
        if idx >= instrument_racks.len() {
            instrument_racks.resize_with(idx + 1, InstrumentRack::default);
        }
        let rack = &mut mixer_racks[idx];
        if !returned.is_empty() {
            rack.on_returns(returned);
        }
        let irack = &mut instrument_racks[idx];
        if !instrument_returned.is_empty() {
            irack.on_returns(instrument_returned);
        }
        rack.poll_requests(Some(&audio.handle));
        irack.poll_requests(&audio.handle);
        rack.ensure_all_sent(&audio.handle, audio.sample_rate);
        irack.ensure_all_sent(&audio.handle, audio.sample_rate);
    }
}
