//! UI 线程 → 渲染线程的参数变化队列。
//!
//! CLAP 宿主没有直接改写参数的 API（clack 0.1 未暴露 `set_value`），参数写入
//! 必须以 `ParamValue` 事件在 process 时交给插件。UI 拖动是高频的，走命令
//! 通道（bounded 会丢）不合适，因此用本队列累积：UI 线程 [`ParamQueue::push`]，
//! 渲染线程每块 [`take_into`](ParamQueue::take_into) 后转成事件。

use std::sync::Mutex;

/// 待应用的参数变化（同 param_id latest-wins）。
#[derive(Default)]
pub struct ParamQueue {
    /// 累积的参数变化；push 时同 id 覆盖（拖动只关心最新值）。
    /// 锁只被 UI 短临界区写、渲染线程每块取一次，无跨线程阻塞等待。
    pending: Mutex<Vec<(u32, f64)>>,
}

impl ParamQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// UI 线程：记录一次参数变化（同 id 覆盖，避免拖动时事件堆积）。
    pub fn push(&self, param_id: u32, value: f64) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        match pending.iter_mut().find(|(id, _)| *id == param_id) {
            Some(slot) => slot.1 = value,
            None => pending.push((param_id, value)),
        }
    }

    /// 渲染线程：把全部待应用变化追加进 `out`（保留两边容量，无分配）。
    pub(crate) fn take_into(&self, out: &mut Vec<(u32, f64)>) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        out.append(&mut pending);
    }

    /// 是否有待应用变化（暂停 flush 前的快速检查）。
    pub fn is_empty(&self) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_same_id_keeps_latest() {
        let q = ParamQueue::new();
        q.push(7, 0.1);
        q.push(7, 0.9);
        q.push(8, 0.5);
        let mut out = Vec::new();
        q.take_into(&mut out);
        assert_eq!(out, vec![(7, 0.9), (8, 0.5)]);
    }

    #[test]
    fn take_into_clears_queue() {
        let q = ParamQueue::new();
        q.push(1, 0.5);
        let mut out = Vec::new();
        q.take_into(&mut out);
        assert_eq!(out.len(), 1);
        out.clear();
        q.take_into(&mut out);
        assert!(out.is_empty());
    }
}
