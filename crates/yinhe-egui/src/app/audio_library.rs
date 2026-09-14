//! UI 侧音频素材库：后台解码 + 峰值数据 + 引擎素材推送。
//!
//! 解码是重活（symphonia 解析 + 可能的重采样），全部在后台线程执行；
//! 结果 `Arc<DecodedAudio>` 同时供 UI（波形峰值绘制）与引擎（片段回放）使用。
//! 引擎 teardown/重建后由 `push_all_to_engine` 重推已有素材。
//!
//! 采样率跟随引擎：设备采样率变化时已解码 PCM 作废，重新解码到新采样率。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc;

use yinhe_audio::{AudioCommand, AudioHandle, DecodedAudio, decode_audio};
use yinhe_core::YinModel;

/// 后台解码结果。
type DecodeResult = (String, Result<Arc<DecodedAudio>, String>);

pub(crate) struct AudioLibrary {
    /// uuid → 已解码素材。
    sources: HashMap<String, Arc<DecodedAudio>>,
    /// 正在后台解码的素材 uuid（避免重复提交）。
    pending: HashSet<String>,
    rx: mpsc::Receiver<DecodeResult>,
    tx: mpsc::Sender<DecodeResult>,
    /// 已解码素材的目标采样率（引擎采样率）。0 = 尚未解码过。
    sample_rate: u32,
    /// 解码失败信息：(素材 uuid 或名称, 错误)。UI 可展示。
    pub errors: Vec<(String, String)>,
}

impl AudioLibrary {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            sources: HashMap::new(),
            pending: HashSet::new(),
            rx,
            tx,
            sample_rate: 0,
            errors: Vec::new(),
        }
    }

    /// 确保模型内所有素材都已解码。
    ///
    /// `target_sample_rate` = 引擎实际采样率；与库不一致时清空重解
    ///（设备切换后引擎重建，PCM 必须匹配新采样率）。
    /// 未开始解码的素材提交到后台线程（提交后线程创建失败会记 submit_error）。
    pub fn ensure_decoded(&mut self, model: &YinModel, target_sample_rate: u32) {
        if target_sample_rate == 0 {
            return;
        }
        if self.sample_rate != target_sample_rate {
            // 采样率变化：已有 PCM 不再匹配，清空重解。
            if self.sample_rate != 0 && !self.sources.is_empty() {
                tracing::info!(
                    "音频素材库采样率变化 {} → {}，重新解码",
                    self.sample_rate,
                    target_sample_rate
                );
            }
            self.sources.clear();
            self.pending.clear();
            self.sample_rate = target_sample_rate;
        }
        for source in &model.audio_sources {
            if self.sources.contains_key(&source.uuid) || self.pending.contains(&source.uuid) {
                continue;
            }
            self.pending.insert(source.uuid.clone());
            let uuid = source.uuid.clone();
            let name = source.name.clone();
            let data = Arc::clone(&source.data);
            let tx = self.tx.clone();
            let spawned = std::thread::Builder::new()
                .name("audio-decode".into())
                .spawn(move || {
                    let result = decode_audio(&data, target_sample_rate)
                        .map(Arc::new)
                        .map_err(|e| format!("{name}: {e}"));
                    let _ = tx.send((uuid, result));
                });
            if let Err(e) = spawned {
                self.pending.remove(&source.uuid);
                tracing::warn!("无法启动音频解码线程: {e}");
            }
        }
    }

    /// 收取后台解码结果；返回本次新到的素材（调用方推给引擎）。
    pub fn poll(&mut self) -> Vec<(String, Arc<DecodedAudio>)> {
        let mut arrived = Vec::new();
        while let Ok((uuid, result)) = self.rx.try_recv() {
            self.pending.remove(&uuid);
            match result {
                Ok(decoded) => {
                    self.sources.insert(uuid.clone(), Arc::clone(&decoded));
                    arrived.push((uuid, decoded));
                }
                Err(e) => {
                    tracing::warn!("音频素材解码失败: {e}");
                    if self.errors.len() >= 16 {
                        self.errors.remove(0);
                    }
                    self.errors.push((uuid, e));
                }
            }
        }
        arrived
    }

    /// 已解码素材（UI 波形绘制用）。未解码完成返回 None。
    pub fn get(&self, uuid: &str) -> Option<&Arc<DecodedAudio>> {
        self.sources.get(uuid)
    }

    /// 素材是否仍在后台解码（UI 显示"解码中"占位）。
    pub fn is_pending(&self, uuid: &str) -> bool {
        self.pending.contains(uuid)
    }

    /// 直接插入已解码素材（录音产物；跳过解码路径）。
    /// 库尚未建立采样率时以该素材采样率为准。
    pub fn insert_decoded(&mut self, uuid: String, decoded: Arc<DecodedAudio>) {
        if self.sample_rate == 0 {
            self.sample_rate = decoded.sample_rate;
        }
        self.pending.remove(&uuid);
        self.sources.insert(uuid, decoded);
    }

    /// 引擎（重）建后把所有已解码素材推给引擎。
    pub fn push_all_to_engine(&self, handle: &AudioHandle) {
        for (uuid, decoded) in &self.sources {
            handle.send(AudioCommand::SetAudioSource {
                uuid: uuid.clone(),
                decoded: Arc::clone(decoded),
            });
        }
    }

    /// 单个新到素材推给引擎。
    pub fn push_to_engine(&self, handle: &AudioHandle, uuid: String, decoded: Arc<DecodedAudio>) {
        handle.send(AudioCommand::SetAudioSource { uuid, decoded });
    }
}

impl Default for AudioLibrary {
    fn default() -> Self {
        Self::new()
    }
}
