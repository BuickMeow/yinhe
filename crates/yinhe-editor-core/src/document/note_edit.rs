//! Note editing operations: add, delete, duplicate, transpose, move, resize.
//!
//! 单音符操作（pencil drag, velocity）在 `note_pencil.rs`。
//!
//! 子模块按主题拆分（每个文件一个主题，测试在 `note_edit/tests/` 同名镜像）：
//! - `add_delete`：增删；
//! - `duplicate`：复制；
//! - `move_resize`：拖动与拉伸；
//! - `transform`：移调 / 翻转 / 变速；
//! - `field_edit`：Info 面板表达式编辑；
//! - `humanize`：人性化随机偏差；
//! - `cleanup`：黑乐谱清理（去重 / 填缝 / 合并）；
//! - `tools`：工具批量生成与切割。

mod add_delete;
mod cleanup;
mod duplicate;
mod field_edit;
mod humanize;
mod move_resize;
mod tools;
mod transform;

#[cfg(test)]
mod tests;

/// 批量编辑的字段（Info 面板选框编辑）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteField {
    /// 力度（0-127）。
    Velocity,
    /// 音符长度 gate（end - start，tick）。
    Gate,
    /// 琴键（0-MAX_KEY）。
    Key,
    /// 起始 tick。
    Tick,
}

/// 翻转方向（选中音符镜像）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlipAxis {
    /// 水平翻转：按选框整体 tick 范围镜像（start/end 互换镜像，gate 不变）。
    Horizontal,
    /// 垂直翻转：按选框整体 key 范围镜像。
    Vertical,
}

/// 人性化参数：三路独立随机偏差的幅度（0 = 该项不动）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HumanizeParams {
    /// 时间：start/end 整体平移 ±timing tick（gate 不变）。
    pub timing: u32,
    /// 力度：velocity ±velocity（clamp 0..=127）。
    pub velocity: u8,
    /// 时值：gate ±gate tick（最短 1 tick）。
    pub gate: u32,
}

impl HumanizeParams {
    /// 三路幅度全为 0：不产生任何修改。
    pub fn is_noop(&self) -> bool {
        self.timing == 0 && self.velocity == 0 && self.gate == 0
    }
}
