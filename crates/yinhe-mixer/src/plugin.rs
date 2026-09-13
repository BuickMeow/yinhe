//! 插件（效果器/乐器）的格式无关抽象：CLAP、VST3 等插件后端共用。
//!
//! 引擎与混音图只依赖本模块类型，不依赖任何具体插件格式 crate：
//! - [`PluginEvent`]：渲染线程喂给插件的事件（各后端转换为原生事件）；
//! - [`InstrumentProcessor`]：乐器处理器（管理线程激活产出、渲染线程独占调用、
//!   回收时 move 回管理线程销毁，与 [`crate::InsertProcessor`] 同一生命周期模型）。

/// 渲染线程输入给插件的单条事件（`time` 为块内 sample offset）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PluginEvent {
    NoteOn {
        time: u32,
        channel: u8,
        key: u8,
        /// 0.0 ~ 1.0（MIDI velocity / 127）。
        velocity: f64,
    },
    NoteOff {
        time: u32,
        channel: u8,
        key: u8,
        velocity: f64,
    },
    /// 掐掉指定键的所有发声。
    NoteChoke { time: u32, channel: u8, key: u8 },
    /// 原始 MIDI 1.0 消息（CC、弯音、ProgramChange 等），最多 3 字节。
    Midi { time: u32, data: [u8; 3] },
    /// 插件参数变化。
    ParamValue {
        time: u32,
        param_id: u32,
        value: f64,
    },
}

/// 乐器插件处理器抽象。
///
/// 生命周期与 [`crate::InsertProcessor`] 一致：管理线程加载/激活产出，
/// 渲染线程独占调用，回收时 move 回管理线程销毁（插件必须在原线程 deactivate）。
pub trait InstrumentProcessor: Send {
    /// 处理一个块：`events` 喂给插件，主端口立体声输出**覆盖**写入
    /// `out_l`/`out_r`（输出不足部分填 0）。`position_samples` 为块起始的
    /// 工程时间（采样数），插件用它对齐内部时钟。
    ///
    /// 实时约束：不得分配内存、不得阻塞、不 panic。处理失败由实现内部
    /// 记录并旁通（本块输出静音）。
    fn process(
        &mut self,
        events: &[PluginEvent],
        out_l: &mut [f32],
        out_r: &mut [f32],
        position_samples: u64,
    );

    /// 清空内部处理状态（envelope、delay 尾音等）。seek 后调用。
    fn reset(&mut self) {}

    /// 插件报告的延迟（采样数），供延迟补偿（PDC）用。
    fn latency_samples(&self) -> u32 {
        0
    }

    /// 回收时还原为具体类型（交还插件实例 deactivate）。
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send>;
}
