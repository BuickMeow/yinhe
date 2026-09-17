//! 引擎状态迁移：模型/音色库应用、chase 控制恢复、传输位置、音符生命周期。
//!
//! 方法按职责拆到四个子模块（均为 `impl AudioEngine`），调用方只依赖
//! `AudioEngine` 的方法而无需关心模块位置。

mod chase;
mod model;
mod notes;
mod transport;
