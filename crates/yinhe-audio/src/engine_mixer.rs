//! 引擎的混音台接线：MixerParams（源通道索引）↔ MixerGraph（dense 索引）映射，
//! insert 命令处理与处理器回收。

use yinhe_mixer::{InsertProcessor, MasterParams, MixerParams, StripParams};

use crate::engine::AudioEngine;

impl AudioEngine {
    /// 全量同步混音台参数（引擎 spawn/工程加载后由 UI 推一次）。
    /// 只推 strip/master 参数；insert 处理器走 Insert* 命令单独进。
    pub(crate) fn set_mixer_params(&mut self, params: MixerParams) {
        self.mixer_params = params;
        let strips = self.dense_strip_params();
        for (dense, p) in strips.into_iter().enumerate() {
            self.mixer.set_strip(dense, p);
        }
        self.mixer.set_master(self.mixer_params.master);
    }

    /// 更新某源通道的 strip（推子/声像/M/S 拖动的高频路径，幂等）。
    pub(crate) fn set_channel_strip(&mut self, channel: u8, params: StripParams) {
        if let Some(slot) = self.mixer_params.channels.get_mut(channel as usize) {
            *slot = params;
        }
        let dense = self.channel_layout.dense_for(channel as usize);
        if dense != u32::MAX {
            self.mixer.set_strip(dense as usize, params);
        }
    }

    pub(crate) fn set_master_params(&mut self, params: MasterParams) {
        self.mixer_params.master = params;
        self.mixer.set_master(params);
    }

    /// 各 dense 通道当前的 strip 参数（resize 重建 strip 状态用）。
    pub(crate) fn dense_strip_params(&self) -> Vec<StripParams> {
        (0..self.channel_set.channel_count())
            .map(|dense| {
                // dense → 源通道：channel_map 反查（仅引擎创建/resize 时调用）。
                let src = self
                    .channel_layout
                    .channel_map()
                    .iter()
                    .position(|&d| d as usize == dense);
                src.map(|s| self.mixer_params.strip(s as u8))
                    .unwrap_or_default()
            })
            .collect()
    }

    /// 源通道 → dense 索引（未激活返回 None）。
    fn dense_of(&self, channel: u8) -> Option<usize> {
        let dense = self.channel_layout.dense_for(channel as usize);
        (dense != u32::MAX).then_some(dense as usize)
    }

    pub(crate) fn insert_add(
        &mut self,
        channel: Option<u8>,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    ) {
        match channel {
            Some(ch) => match self.dense_of(ch) {
                Some(dense) => self.mixer.insert_insert(dense, slot, processor),
                // 通道未激活（模型无音轨用此通道）：处理器无处安放，直接退回。
                None => self.insert_returns.push(processor),
            },
            None => self.mixer.insert_master_insert(slot, processor),
        }
    }

    pub(crate) fn insert_remove(&mut self, channel: Option<u8>, slot: usize) {
        let removed = match channel {
            Some(ch) => self
                .dense_of(ch)
                .and_then(|dense| self.mixer.remove_insert(dense, slot)),
            None => self.mixer.remove_master_insert(slot),
        };
        if let Some(p) = removed {
            self.insert_returns.push(p);
        }
    }

    pub(crate) fn insert_replace(
        &mut self,
        channel: Option<u8>,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    ) {
        let old = match channel {
            Some(ch) => self
                .dense_of(ch)
                .and_then(|dense| self.mixer.replace_insert(dense, slot, processor)),
            None => self.mixer.replace_master_insert(slot, processor),
        };
        if let Some(p) = old {
            self.insert_returns.push(p);
        }
    }

    /// 取出待回收的 insert 处理器（renderer 每轮命令处理后调用，送回 UI 线程）。
    pub(crate) fn drain_insert_returns(&mut self) -> Vec<Box<dyn InsertProcessor>> {
        std::mem::take(&mut self.insert_returns)
    }
}
