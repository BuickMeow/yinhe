//! GPU compute cull state: per-key note buffers + indirect dispatch.
//!
//! Architecture: each key (0..KEY_COUNT) owns its own `all_notes` (input),
//! `visible_notes` (output), and a per-key draw-args buffer. The cull
//! dispatch loops over keys; each key's visible capacity equals its all-notes
//! capacity, so there is no global visible-note cap.
//!
//! Memory: all_notes + visible_notes ≈ 2 × total notes × 16B (worst case:
//! minimum zoom, every note visible). H2O.mid (13.8M) ≈ 374MB; 100M ≈ 3.2GB.
//!
//! 模块拆分（无 mod.rs 的现代写法）：
//! - `bucket.rs`：KeyBucketIndex —— CPU tick 桶索引（每帧只 dispatch 可见 chunk）。
//! - `state.rs`：CullState —— GPU compute 剔除状态机 + 视口 tick 范围计算。
//! - `tests.rs`：集成测试（headless wgpu device）。

mod bucket;
mod state;

#[cfg(test)]
mod tests;

pub(crate) use bucket::KeyBucketIndex;
pub(crate) use state::CullState;

#[cfg(test)]
pub(crate) use state::visible_tick_range;
