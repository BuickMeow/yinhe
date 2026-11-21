//! GPU compute cull state: per-key note buffers + indirect dispatch.
//!
//! Architecture: each key (0..KEY_COUNT) owns its own `all_notes` (input),
//! `visible_notes` (output), and a per-key draw-args buffer. The cull
//! dispatch loops over keys; each key's visible capacity equals its all-notes
//! capacity, so there is no global visible-note cap.
//!
//! Memory: all_notes 12B/音符 + visible 4B 索引/槽（稀疏槽按 256 对齐）。
//! 100M 音符约 1.2GB + 0.4GB；8 档 LOD 摘要另计（40M 实测约 49MB）。
//!
//! 模块拆分（无 mod.rs 的现代写法）：
//! - `bucket.rs`：KeyBucketIndex —— CPU tick 桶索引（每帧只 dispatch 可见 chunk）。
//! - `state.rs`：CullState —— GPU compute 剔除状态机 + 视口 tick 范围计算。
//! - `tests.rs`：集成测试（headless wgpu device）。

mod bucket;
mod state;
mod summary;

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod bench;

pub(crate) use bucket::KeyBucketIndex;
pub(crate) use state::CullState;

#[cfg(test)]
pub(crate) use state::visible_tick_range;
