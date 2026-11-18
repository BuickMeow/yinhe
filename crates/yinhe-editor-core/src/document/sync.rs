use super::*;

impl Document {
    /// 每帧预算内增量刷新滚动条密度预览。
    ///
    /// 维护两份数据：`density` 跟随 `track_visible`（AR/通道 AR 用），
    /// `density_pr` 跟随 PR 专属显隐（`track_pianoroll_visible ∪ 主音轨`，
    /// 与 PR 画布一致）。预算对半分给两份，编辑时各自只重扫脏 key。
    pub fn sync_density(&mut self, budget: std::time::Duration) {
        let Self {
            data,
            edit,
            density,
            density_pr,
            ..
        } = self;
        let half = budget / 2;
        density.sync(&data.model, &edit.track_visible, half);
        // 与 PR 画布同一份显隐规则（EditState::pr_visible）。
        let pr_visible = edit.pr_visible();
        density_pr.sync(&data.model, &pr_visible, half);
    }

    /// `sync_track_caches` with explicit dark/light theme for Conductor color.
    pub fn sync_track_caches_with_dark(&mut self, is_dark: bool) {
        let conductor_color = if is_dark {
            let c = yinhe_theme::base::BaseColors::DARK.text;
            [
                c.r as f32 / 255.0,
                c.g as f32 / 255.0,
                c.b as f32 / 255.0,
                1.0,
            ]
        } else {
            let c = yinhe_theme::base::BaseColors::LIGHT.text;
            [
                c.r as f32 / 255.0,
                c.g as f32 / 255.0,
                c.b as f32 / 255.0,
                1.0,
            ]
        };
        self.sync_track_caches_with_conductor_color(conductor_color);
    }

    /// `sync_track_caches` with precise Conductor color (`text_primary` from theme).
    pub fn sync_track_caches_with_conductor_color(&mut self, conductor_color: [f32; 4]) {
        self.ensure_audio_targets();
        self.edit.track_cache.rebuild(&self.data, conductor_color);
        let num_tracks = self.data.model.tracks.len();
        while self.edit.track_visible.len() < num_tracks {
            self.edit.track_visible.push(true);
        }
        while self.edit.track_pianoroll_visible.len() < num_tracks {
            self.edit.track_pianoroll_visible.push(true);
        }
        while self.edit.track_locked.len() < num_tracks {
            self.edit.track_locked.push(false);
        }
        while self.edit.track_overrides.len() < num_tracks {
            self.edit.track_overrides.push(Default::default());
        }
        while self.edit.track_visible.len() > num_tracks {
            self.edit.track_visible.pop();
        }
        while self.edit.track_pianoroll_visible.len() > num_tracks {
            self.edit.track_pianoroll_visible.pop();
        }
        while self.edit.track_locked.len() > num_tracks {
            self.edit.track_locked.pop();
        }
        while self.edit.track_overrides.len() > num_tracks {
            self.edit.track_overrides.pop();
        }
        // 轨道结构/种类变化后同步 AP 表（共享 MIDI 通道 -> AP）。
        self.ensure_aps();
    }

    /// 设置 Track 的三态听感。Track solo 是内容门；进入 Track solo 时清空
    /// 所有 Channel solo，避免两层 solo 叠加。
    pub fn set_track_audibility(&mut self, idx: usize, audibility: Audibility) -> bool {
        let Some(ov) = self.edit.track_overrides.get_mut(idx) else {
            return false;
        };
        if ov.audibility() == audibility {
            return false;
        }
        ov.set_audibility(audibility);
        if audibility == Audibility::Soloed {
            let _ = self.clear_channel_solos();
        }
        true
    }

    /// 设置 Channel 的三态听感。进入 Channel solo 时清空所有 Track solo。
    pub fn set_channel_audibility(&mut self, node: MixerChannelId, audibility: Audibility) -> bool {
        if self.mixer.node_strip(node).audibility() == audibility {
            return false;
        }
        let mut strip = self.mixer.node_strip(node);
        strip.set_audibility(audibility);
        if !self.mixer.set_node_strip(node, strip) {
            return false;
        }
        if audibility == Audibility::Soloed {
            self.clear_track_solos();
        }
        self.mark_mixer_dirty();
        true
    }

    /// 清空所有 Channel solo，返回被改动的通道（用于推引擎）。
    pub fn clear_channel_solos(&mut self) -> Vec<MixerChannelId> {
        let mut changed = Vec::new();
        for node in self.mixer.nodes_mut() {
            if node.solo {
                node.solo = false;
                changed.push(match node.source {
                    yinhe_types::ChannelSource::Midi(ch) => MixerChannelId::Midi(ch),
                    yinhe_types::ChannelSource::Audio(ch) => MixerChannelId::Audio(ch),
                    yinhe_types::ChannelSource::Track(_) => match node.id {
                        yinhe_types::ChannelId::Lc(id) => MixerChannelId::TrackLc(id),
                        _ => MixerChannelId::Master,
                    },
                    yinhe_types::ChannelSource::Ap(_) | yinhe_types::ChannelSource::Master => {
                        MixerChannelId::Master
                    }
                });
            }
        }
        if !changed.is_empty() {
            self.mark_mixer_dirty();
        }
        changed
    }

    fn clear_track_solos(&mut self) {
        for ov in &mut self.edit.track_overrides {
            if ov.soloed {
                ov.soloed = false;
            }
        }
    }

    /// 合并 mute/solo 状态为音频引擎用的 skip mask。
    ///
    /// 规则：有任意 solo 时，只有 soloed 轨道不 skip；无 solo 时，muted 轨道 skip。
    /// 音频引擎读此 mask 过滤音符和自动化事件。
    pub fn compute_skip_mask(&self) -> Vec<bool> {
        let has_solo = self.edit.track_overrides.iter().any(|t| t.soloed);
        self.edit
            .track_overrides
            .iter()
            .map(|ov| if has_solo { !ov.soloed } else { ov.muted })
            .collect()
    }

    /// 保存前把运行时 mute/solo 状态写回 model.tracks，
    /// 使 `sync_mapping_file` 能读到正确的持久化值。
    ///
    /// `TrackData` 是 `Arc`，用 `Arc::make_mut` 做 copy-on-write，
    /// 不影响 undo/redo 快照持有的旧 Arc。
    ///
    /// 只复制真正有变化的轨道：保存大工程时渲染线程通常持有同一份
    /// `Arc`，无条件 `make_mut` 会为每个轨道复制整条 `TrackData`。
    pub fn sync_overrides_to_model(&mut self) {
        let changed: Vec<usize> = self
            .edit
            .track_overrides
            .iter()
            .enumerate()
            .filter_map(|(i, ov)| {
                let td = self.data.model.tracks.get(i)?;
                (td.muted != ov.muted || td.soloed != ov.soloed).then_some(i)
            })
            .collect();
        if changed.is_empty() {
            return;
        }
        let model = Arc::make_mut(&mut self.data.model);
        for i in changed {
            let ov = &self.edit.track_overrides[i];
            if let Some(td) = model.tracks.get_mut(i) {
                let td = Arc::make_mut(td);
                td.muted = ov.muted;
                td.soloed = ov.soloed;
            }
        }
    }
}
