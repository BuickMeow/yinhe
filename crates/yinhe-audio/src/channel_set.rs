//! 自有通道组：分通道渲染版 ChannelGroup。
//!
//! xsynth 的 `ChannelGroup` 只提供「所有通道混成一路立体声」的 `read_samples`，
//! 通道字段私有，无法拿到每通道音频。混音台需要按通道做增益/声像/静音/insert，
//! 因此用公开的 `VoiceChannel` 自建同等结构：事件缓存 + 双 rayon 池 +
//! 分通道渲染进混音台的 planar 缓冲。事件语义（`SynthEvent`）与
//! `ChannelGroup` 完全对齐，上层调用点只需换类型名。

use std::sync::Arc;

use rayon::prelude::*;
use xsynth_core::AudioPipe;
use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent, VoiceChannel};
use xsynth_core::channel_group::{ChannelGroupConfig, SynthEvent, SynthFormat, ThreadCount};

use yinhe_mixer::ChannelBuffers;

/// 与 xsynth ChannelGroup 相同的事件缓存阈值：超过则在下次 send_event 时 flush。
const MAX_EVENT_CACHE_SIZE: u32 = 1024 * 1024;

/// 分通道渲染的通道组（dense 通道索引，与 `ChannelLayout` 一致）。
pub(crate) struct ChannelSet {
    channels: Box<[VoiceChannel]>,
    channel_events_cache: Box<[Vec<ChannelAudioEvent>]>,
    /// 每通道交错立体声暂存（read_samples 输出交错，deinterleave 进 planar 缓冲）。
    scratches: Box<[Vec<f32>]>,
    cached_event_count: u32,
    /// 跨通道并行池（AUTO_PER_CHANNEL 时存在）。
    thread_pool: Option<rayon::ThreadPool>,
    audio_params: xsynth_core::AudioStreamParams,
}

impl ChannelSet {
    /// 与 `ChannelGroup::new` 相同的配置入口；`max_frames` 决定暂存缓冲大小。
    pub(crate) fn new(config: ChannelGroupConfig, max_frames: usize) -> Self {
        let channel_pool = match config.parallelism.key {
            ThreadCount::None => None,
            ThreadCount::Auto => Some(Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .build()
                    .unwrap_or_else(|e| panic!("yinhe: 创建 key 渲染线程池失败: {e}")),
            )),
            ThreadCount::Manual(threads) => Some(Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap_or_else(|e| panic!("yinhe: 创建 key 渲染线程池失败: {e}")),
            )),
        };
        let group_pool = match config.parallelism.channel {
            ThreadCount::None => None,
            ThreadCount::Auto => Some(
                rayon::ThreadPoolBuilder::new()
                    .build()
                    .unwrap_or_else(|e| panic!("yinhe: 创建通道渲染线程池失败: {e}")),
            ),
            ThreadCount::Manual(threads) => Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap_or_else(|e| panic!("yinhe: 创建通道渲染线程池失败: {e}")),
            ),
        };

        let channel_count = match config.format {
            SynthFormat::Midi => 16,
            SynthFormat::Custom { channels } => channels,
        } as usize;

        let mut channels = Vec::with_capacity(channel_count);
        let mut caches = Vec::with_capacity(channel_count);
        let mut scratches = Vec::with_capacity(channel_count);
        for _ in 0..channel_count {
            channels.push(VoiceChannel::new(
                config.channel_init_options,
                config.audio_params,
                channel_pool.clone(),
            ));
            caches.push(Vec::new());
            scratches.push(vec![0.0; max_frames * 2]);
        }

        Self {
            channels: channels.into_boxed_slice(),
            channel_events_cache: caches.into_boxed_slice(),
            scratches: scratches.into_boxed_slice(),
            cached_event_count: 0,
            thread_pool: group_pool,
            audio_params: config.audio_params,
        }
    }

    /// 通道数（dense）。
    pub(crate) fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// 与 `ChannelGroup::send_event` 相同语义：音频事件进缓存（渲染前 flush），
    /// 配置事件直发。
    pub(crate) fn send_event(&mut self, event: SynthEvent) {
        match event {
            SynthEvent::Channel(channel, event) => match event {
                ChannelEvent::Audio(e) => {
                    if let Some(cache) = self.channel_events_cache.get_mut(channel as usize) {
                        cache.push(e);
                        self.cached_event_count += 1;
                        if self.cached_event_count > MAX_EVENT_CACHE_SIZE {
                            self.flush_events();
                        }
                    }
                }
                ChannelEvent::Config(_) => {
                    if let Some(channel) = self.channels.get_mut(channel as usize) {
                        channel.process_event(event);
                    }
                }
            },
            SynthEvent::AllChannels(event) => match event {
                ChannelEvent::Audio(e) => {
                    for cache in self.channel_events_cache.iter_mut() {
                        cache.push(e);
                    }
                    self.cached_event_count += self.channel_events_cache.len() as u32;
                    if self.cached_event_count > MAX_EVENT_CACHE_SIZE {
                        self.flush_events();
                    }
                }
                ChannelEvent::Config(_) => {
                    for channel in self.channels.iter_mut() {
                        channel.process_event(event.clone());
                    }
                }
            },
        }
    }

    /// 把所有缓存事件推入各通道（渲染前调用；超阈值时 send_event 也会触发）。
    fn flush_events(&mut self) {
        if self.cached_event_count == 0 {
            return;
        }
        match self.thread_pool.as_ref() {
            Some(pool) => {
                let channels = &mut self.channels;
                let caches = &mut self.channel_events_cache;
                pool.install(move || {
                    channels.par_iter_mut().zip(caches.par_iter_mut()).for_each(
                        |(channel, events)| {
                            channel.push_events_iter(events.drain(..).map(ChannelEvent::Audio));
                        },
                    );
                });
            }
            None => {
                for (channel, events) in self
                    .channels
                    .iter_mut()
                    .zip(self.channel_events_cache.iter_mut())
                {
                    channel.push_events_iter(events.drain(..).map(ChannelEvent::Audio));
                }
            }
        }
        self.cached_event_count = 0;
    }

    /// 渲染 [`offset_frames`, `offset_frames + frames`) 区间：每通道事件 flush +
    /// 渲染进交错暂存 + deinterleave 进混音台 planar 缓冲（覆盖写，调用方无需清零）。
    ///
    /// `buffers.len()` 必须等于通道数（混音台按 compacted 通道数创建）。
    pub(crate) fn render_segment(
        &mut self,
        buffers: &mut [ChannelBuffers],
        offset_frames: usize,
        frames: usize,
    ) {
        debug_assert_eq!(buffers.len(), self.channels.len());
        if frames == 0 {
            return;
        }
        let interleaved_len = frames * 2;
        // render_segment 覆盖所有通道（含无事件通道，render 输出静音），
        // 因此 flush 一步到位、缓存计数清零。
        self.cached_event_count = 0;

        let channels = &mut self.channels;
        let caches = &mut self.channel_events_cache;
        let scratches = &mut self.scratches;

        // 并行/串行两路共用的单通道渲染闭包（参数类型抽别名，过 clippy type_complexity）。
        type ChannelItem<'a> = (&'a mut VoiceChannel, &'a mut Vec<ChannelAudioEvent>);
        type OutputItem<'a> = (&'a mut Vec<f32>, &'a mut ChannelBuffers);
        let render_one =
            move |((channel, events), (scratch, buf)): (ChannelItem<'_>, OutputItem<'_>)| {
                channel.push_events_iter(events.drain(..).map(ChannelEvent::Audio));
                channel.read_samples(&mut scratch[..interleaved_len]);
                for (i, s) in scratch[..interleaved_len].chunks_exact(2).enumerate() {
                    buf.left[offset_frames + i] = s[0];
                    buf.right[offset_frames + i] = s[1];
                }
            };

        match self.thread_pool.as_ref() {
            Some(pool) => {
                pool.install(|| {
                    channels
                        .par_iter_mut()
                        .zip(caches.par_iter_mut())
                        .zip(scratches.par_iter_mut().zip(buffers.par_iter_mut()))
                        .for_each(render_one);
                });
            }
            None => {
                for item in channels
                    .iter_mut()
                    .zip(caches.iter_mut())
                    .zip(scratches.iter_mut().zip(buffers.iter_mut()))
                {
                    render_one(item);
                }
            }
        }
    }

    /// 暂存缓冲按新块长重建（引擎块长变化时，至多一次）。
    pub(crate) fn resize_scratches(&mut self, max_frames: usize) {
        for s in self.scratches.iter_mut() {
            s.resize(max_frames * 2, 0.0);
        }
    }

    /// 活跃 voice 总数（所有通道求和）。
    pub(crate) fn voice_count(&self) -> u64 {
        self.channels
            .iter()
            .map(|c| c.get_channel_stats().voice_count())
            .sum()
    }

    #[allow(dead_code)] // 调试/测试用
    pub(crate) fn stream_params(&self) -> &xsynth_core::AudioStreamParams {
        &self.audio_params
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xsynth_core::channel::ChannelInitOptions;
    use xsynth_core::channel_group::{ParallelismOptions, SynthFormat};
    use xsynth_core::{AudioStreamParams, ChannelCount};

    fn make_set(channels: u32) -> ChannelSet {
        ChannelSet::new(
            ChannelGroupConfig {
                channel_init_options: ChannelInitOptions {
                    fade_out_killing: true,
                },
                format: SynthFormat::Custom { channels },
                audio_params: AudioStreamParams {
                    sample_rate: 44100,
                    channels: ChannelCount::Stereo,
                },
                parallelism: ParallelismOptions {
                    channel: ThreadCount::None,
                    key: ThreadCount::None,
                },
            },
            512,
        )
    }

    #[test]
    fn silent_channels_render_zero_planar() {
        let mut set = make_set(2);
        let mut buffers = vec![
            ChannelBuffers {
                left: vec![1.0; 8],
                right: vec![1.0; 8],
            },
            ChannelBuffers {
                left: vec![1.0; 8],
                right: vec![1.0; 8],
            },
        ];
        set.render_segment(&mut buffers, 0, 8);
        for b in &buffers {
            assert!(b.left.iter().all(|&v| v == 0.0));
            assert!(b.right.iter().all(|&v| v == 0.0));
        }
    }

    #[test]
    fn segment_offset_writes_disjoint_range() {
        let mut set = make_set(1);
        let mut buffers = vec![ChannelBuffers {
            left: vec![7.0; 8],
            right: vec![7.0; 8],
        }];
        // 只写后半段，前半段保持原值（调用方按段覆盖）。
        set.render_segment(&mut buffers, 4, 4);
        assert_eq!(&buffers[0].left[..4], &[7.0; 4]);
        assert_eq!(&buffers[0].left[4..], &[0.0; 4]);
    }

    #[test]
    fn out_of_range_channel_event_ignored() {
        let mut set = make_set(1);
        set.send_event(SynthEvent::Channel(
            5,
            ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key: 60, vel: 100 }),
        ));
        assert_eq!(set.voice_count(), 0);
    }
}
