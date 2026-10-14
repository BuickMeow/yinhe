//! 通道 AR（MIX 主区域）视图。
//!
//! 与轨道 AR（`arrange`）的本质差异：以**通道为单位**（MIDI 源通道 A01..P16
//! 与音频通道 Audio-XX），而不是以轨道为单位。
//! - 折叠：通道内所有轨道的音符在 GPU 层合并画在通道行上（统一通道色）；
//! - 展开：通道行变窄作总览，下方平铺「轨道音符行 + 轨道自动化行」；
//! - Conductor 行显示 Tempo（直编，与原 AR 一致）。
//!
//! 编辑能力对齐原 AR：选框/移动/橡皮擦/自动化锚点编辑/音频片段编辑。

mod groups;
mod rows;

pub(crate) use groups::{CaChannel, CaChannelGroup, CaGroups};
pub(crate) use rows::{CaRow, CaRowLayout};
