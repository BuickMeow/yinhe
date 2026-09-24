//! 保存/加载进度回调类型。

/// 保存/加载的进度阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YinProgressStage {
    /// 保存：检查桶序（乱序桶兜底排序）
    Collect,
    /// 保存：KEY_COUNT 路归并 + 列编码（可连续汇报）
    Sort,
    /// 保存：zstd 压缩（6 个流）
    Compress,
    /// 加载：zstd 解压（6 个流）
    Decompress,
    /// 加载：按 key 分桶还原音符
    Rebuild,
    /// 加载：桶内按 start 排序
    Resort,
}

/// 进度回调载荷：阶段 + 阶段内进度 0.0~1.0。
#[derive(Clone, Copy, Debug)]
pub struct YinProgress {
    pub stage: YinProgressStage,
    pub fraction: f32,
}

pub(crate) fn progress(
    on_progress: &mut dyn FnMut(YinProgress),
    stage: YinProgressStage,
    fraction: f32,
) {
    on_progress(YinProgress { stage, fraction });
}
