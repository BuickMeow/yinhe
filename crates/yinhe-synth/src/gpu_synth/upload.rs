//! 音色库登记与采样拼接上传（GPU 常驻缓冲的构建与预热）。

use super::*;

impl GpuSynth {
    /// 加载一个 dense 通道的音色库列表（多文件 = 多个 (bank, preset) 条目，
    /// ProgramChange 在它们之间切换）。可多次调用（逐通道加载）。
    ///
    /// 只登记 key map，不上传样本——全部通道加载完成后由调用方调一次
    /// [`finish_soundfont_load`](Self::finish_soundfont_load) 统一上传
    /// （逐通道上传会退化成 O(n²) 全量重传）。
    /// `MAX_CHANNELS`（32）是 GPU 侧的 dense 槽位上限（2 个 MIDI 端口）；
    /// `dense >= MAX_CHANNELS` 返回错误（不支持折叠复用槽位）。
    pub fn load_dense_soundfonts(
        &mut self,
        dense: u32,
        paths: &[std::path::PathBuf],
    ) -> Result<(), String> {
        self.load_dense_soundfonts_many(&[dense], paths)
    }

    /// 一组 dense 槽位共享同一份 key map（多库只合并一次、Arc 共享；
    /// 逐通道调用会重复深拷贝合并，且 `sample_paths` 重复累积）。
    pub fn load_dense_soundfonts_many(
        &mut self,
        denses: &[u32],
        paths: &[std::path::PathBuf],
    ) -> Result<(), String> {
        let maps = load_key_maps_merged(paths, self.sample_rate, self.interpolation)?;
        let mut any = false;
        for &dense in denses {
            let slot = dense as usize;
            if slot >= MAX_CHANNELS {
                continue;
            }
            any = true;
            self.port_key_maps[slot] = Arc::clone(&maps);
            self.channel_port[slot] = slot as u8;
        }
        if any {
            // 每通道只累加一次路径（rebuild_sample_upload 内部去重）
            self.sample_paths.extend(paths.iter().cloned());
        }
        Ok(())
    }

    /// 全部通道音色加载完成后调用一次：把样本统一上传 GPU。
    pub fn finish_soundfont_load(&mut self) {
        self.rebuild_sample_upload();
    }

    /// 预热 GPU 缓冲与管线（加载阶段调用，`finish_soundfont_load` 之后）：
    /// 按最大 voice 容量与段长一次性分配并跑一次哑渲染，
    /// 播放中不再扩容重建、首块也不再触发 GPU 冷启动。
    pub fn prewarm(&mut self, frames: u32) {
        let sample_rate = self.sample_rate;
        self.renderer.prewarm(frames, sample_rate);
    }

    /// 把所有 port 的采样按 Arc 身份去重后拼成大块上传 GPU。
    /// 拼接结果按"音色库路径集合 + 采样率"缓存（只保留最近一份），
    /// 引擎重建导致的重复加载直接复用，跳过 500MB 级重拼。
    fn rebuild_sample_upload(&mut self) {
        let mut paths = self.sample_paths.clone();
        paths.sort();
        paths.dedup();
        let key = (paths, self.sample_rate);

        // 缓存命中：复用拼接数据（样本 Arc 与解析缓存共享，offsets 指针一致）
        if let Some((data, offsets)) = cached_sample_bundle(&key) {
            let mb = data.len() as f64 * 4.0 / (1024.0 * 1024.0);
            self.sample_offsets = offsets;
            self.renderer.upload_samples(data);
            eprintln!("[gpu] 采样拼接命中缓存（{mb:.0}MB，跳过重拼）");
            return;
        }

        // 未命中：按 Arc 身份去重 + 统计总长后一次性预分配拼接
        let t = std::time::Instant::now();
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut unique: Vec<&Arc<[f32]>> = Vec::new();
        // ptr → is_stereo（同一 Arc 的采样布局一致，加载期决定）
        let mut stereo: HashMap<usize, bool> = HashMap::new();
        for entries in &self.port_key_maps {
            for entry in entries.iter() {
                for key_layers in &entry.map {
                    for info in key_layers {
                        let ptr = info.sample_data.as_ptr() as usize;
                        if seen.insert(ptr) {
                            unique.push(&info.sample_data);
                        }
                        stereo.entry(ptr).or_insert(info.is_stereo);
                    }
                }
            }
        }
        let mut data: Vec<f32> = Vec::with_capacity(unique.iter().map(|s| s.len()).sum());
        let mut offsets: HashMap<usize, (u32, u32)> = HashMap::with_capacity(unique.len());
        for sample in &unique {
            let ptr = sample.as_ptr() as usize;
            let offset = data.len() as u32;
            // **帧数**而非元素数：schedule.rs 的 sample_length/info.offset 都是
            // 帧语义（此前立体声样本这里按元素存，sample_length 偏大一倍，
            // 播放尾巴越界读到相邻采样）。
            let scale = 1 + u32::from(*stereo.get(&ptr).unwrap_or(&false));
            let len = sample.len() as u32 / scale;
            data.extend_from_slice(sample);
            // 每个样本起始对齐到偶数元素：shader 用 vec2 视图一次读 (l,r)
            //（立体声）或相邻两元素（单声道），pair 不得跨样本错位。
            if data.len() % 2 != 0 {
                data.push(0.0);
            }
            offsets.insert(ptr, (offset, len));
        }
        let mb = data.len() as f64 * 4.0 / (1024.0 * 1024.0);
        let chunk_count = data.len().div_ceil(crate::synth::types::CHUNK_SIZE);
        let data = Arc::new(data);
        self.sample_offsets = offsets.clone();
        store_sample_bundle(
            key,
            SampleBundle {
                data: Arc::clone(&data),
                offsets,
            },
        );
        self.renderer.upload_samples(data);
        eprintln!(
            "[gpu] 采样拼接={:?}（{chunk_count} 个 chunk，{mb:.0}MB），已缓存复用",
            t.elapsed()
        );
    }
}
