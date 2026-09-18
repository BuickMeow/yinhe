//! 进程级音色库缓存：key map 解析结果 + 采样拼接数据。

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use crate::sf_parser;

/// 进程级音色库解析缓存：key = (路径, 目标采样率)。
/// 反复打开/切换工程不再重复解析（每次约 3-4s）；样本 `Arc` 跨引擎共享，
/// 内存只存一份。缓存常驻（音色库条目数量有限）。
type KeyMapCacheKey = (std::path::PathBuf, u32, u32);
type KeyMapCacheValue = Arc<Vec<sf_parser::KeyMapEntry>>;
static KEY_MAP_CACHE: LazyLock<Mutex<HashMap<KeyMapCacheKey, KeyMapCacheValue>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 预热进程级解析缓存（worker 线程调用）：未命中才解析。
/// 音频线程随后加载同一音色库时直接命中缓存，不再在音频线程里解析（3-4s）。
pub fn prefetch_key_maps(
    path: &std::path::Path,
    sample_rate: u32,
    interp: u32,
) -> Result<(), String> {
    let key = (path.to_path_buf(), sample_rate, interp);
    {
        let cache = KEY_MAP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if cache.contains_key(&key) {
            return Ok(());
        }
    }
    let t = std::time::Instant::now();
    let built = Arc::new(sf_parser::build_key_maps(path, sample_rate, interp)?);
    eprintln!(
        "[synth] worker 预解析音色库={:?}：{}",
        t.elapsed(),
        path.display()
    );
    KEY_MAP_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_insert(built);
    Ok(())
}

/// 加载一组音色库的 key map 并合并为一份（Arc 共享）：单库直接复用缓存
/// Arc（零克隆），多库拼接一次。供 CPU/GPU 后端按通道登记。
pub(crate) fn load_key_maps_merged(
    paths: &[std::path::PathBuf],
    sample_rate: u32,
    interp: u32,
) -> Result<Arc<Vec<sf_parser::KeyMapEntry>>, String> {
    if paths.len() == 1 {
        return load_key_maps(&paths[0], sample_rate, interp);
    }
    let mut merged: Vec<sf_parser::KeyMapEntry> = Vec::new();
    for path in paths {
        merged.extend(load_key_maps(path, sample_rate, interp)?.iter().cloned());
    }
    Ok(Arc::new(merged))
}

/// 加载（未命中则解析并缓存）一个音色库的 key map。
/// GPU 与 yinhe CPU 的 `load_dense_soundfonts` 共用（音频线程只查缓存）。
pub(crate) fn load_key_maps(
    path: &std::path::Path,
    sample_rate: u32,
    interp: u32,
) -> Result<Arc<Vec<sf_parser::KeyMapEntry>>, String> {
    let key = (path.to_path_buf(), sample_rate, interp);
    let cached = {
        let cache = KEY_MAP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(&key).cloned()
    };
    if let Some(arc) = cached {
        // 命中不打印：启动时按通道逐个查询会刷屏几十行（命中本身零成本）。
        return Ok(arc);
    }
    let t = std::time::Instant::now();
    let built = Arc::new(sf_parser::build_key_maps(path, sample_rate, interp)?);
    eprintln!(
        "[synth] 音色库解析（未命中缓存）={:?}：{}",
        t.elapsed(),
        path.display()
    );
    // 并发下可能有别的线程先插入：取缓存内实际条目（or_insert），
    // 保证所有实例共享同一份样本指针（拼接缓存的 offsets 依赖指针）。
    Ok(KEY_MAP_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_insert(built)
        .clone())
}

/// 采样拼接结果缓存（只保留最近一份）：key = 排序去重后的音色库路径集合 + 采样率。
/// 反复加载相同音色库的工程（引擎重建会新建 GpuSynth）不再重新拼接
/// 500MB 级连续缓冲（~2.3s）；`data` 为 Arc 共享，与渲染器引用同一份内存
/// （总内存不增加，换音色库组合时旧数据自然释放）。
pub(super) type SampleBundleKey = (Vec<std::path::PathBuf>, u32);
/// 拼接后的连续采样数据（Arc 与渲染器共享同一份内存）。
pub(super) type SampleData = Arc<Vec<f32>>;
/// 按样本 Arc 身份（指针 as usize）索引的 (offset, len) 表。
pub(super) type SampleOffsets = HashMap<usize, (u32, u32)>;
pub(super) struct SampleBundle {
    pub(super) data: SampleData,
    pub(super) offsets: SampleOffsets,
}
static SAMPLE_BUNDLE_CACHE: LazyLock<Mutex<Option<(SampleBundleKey, SampleBundle)>>> =
    LazyLock::new(|| Mutex::new(None));

/// 命中拼接缓存时取回 (data, offsets)；未命中返回 None。
pub(super) fn cached_sample_bundle(key: &SampleBundleKey) -> Option<(SampleData, SampleOffsets)> {
    let cache = SAMPLE_BUNDLE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match &*cache {
        Some((k, bundle)) if k == key => Some((Arc::clone(&bundle.data), bundle.offsets.clone())),
        _ => None,
    }
}

/// 存入拼接缓存（只保留最近一份，旧数据自然释放）。
pub(super) fn store_sample_bundle(key: SampleBundleKey, bundle: SampleBundle) {
    let mut cache = SAMPLE_BUNDLE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *cache = Some((key, bundle));
}
