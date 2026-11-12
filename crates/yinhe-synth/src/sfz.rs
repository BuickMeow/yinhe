//! 自有 SFZ 解析器：替换 `xsynth-soundfonts::sfz`。
//!
//! 动机：xsynth 的 SFZ 解析只认 39 个 opcode，其余静默丢弃。而真实音色库
//! （如 Starry Studio Grand）大量使用 `xfin_*`（分层 CC 门控）、`trigger=release`
//! （释放采样）、`seq_*`（round-robin）、`pitch_random`、`amplfo_*`——这些恰恰是
//! "听起来像真乐器"的关键。详见 `docs/spec-sfz-coverage.md`。
//!
//! 模块结构（不使用 `mod.rs`）：
//! - `types`：对外数据模型
//! - `lexer`：行级词法（`#include` / `#define` / 标签 / opcode）
//! - `opcodes`：opcode → 类型化字段（含 `xfin_locc20` 这类编号后缀）
//! - `region`：层级继承并落成 `SfzRegion`

pub mod lexer;
pub mod opcodes;
pub mod region;
pub mod types;

pub use region::parse;
pub use types::{Ampeg, CcGate, GateDir, Lfo, LoopMode, OnGate, SfzFile, SfzRegion, Trigger};
