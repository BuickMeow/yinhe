//! 人性化：三路随机偏差（时间 / 力度 / 时值）。

use std::sync::Arc;

use crate::batch_ops;
use crate::document::Document;
use crate::history::{NoteDelta, UndoAction};

use super::HumanizeParams;

impl Document {
    /// 人性化：给选中音符注入三路独立随机偏差（时间平移 / 力度 / 时值），
    /// 一键操作、一次撤销。返回 `None` 表示没有音符被修改。
    ///
    /// - 时间：start/end 整体平移 ±timing tick（gate 不变，start 不越过 0）；
    /// - 力度：velocity ±velocity（clamp 0..=127）；
    /// - 时值：gate ±gate tick（最短 1 tick）。
    pub fn humanize_selected(&mut self, params: HumanizeParams) -> Option<UndoAction> {
        // xorshift 状态不能为 0（吸收态），`| 1` 兜底。
        let mut rng = uuid::Uuid::new_v4().as_u128() as u64 | 1;
        self.humanize_selected_with_rng(params, &mut rng)
    }

    /// `humanize_selected` 的确定性版本：随机状态由调用方提供（测试用固定种子）。
    pub(super) fn humanize_selected_with_rng(
        &mut self,
        params: HumanizeParams,
        rng: &mut u64,
    ) -> Option<UndoAction> {
        if self.edit.selected.is_empty() || params.is_noop() {
            return None;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let originals = batch_ops::remove_selected(model, &self.edit.selected);
        if originals.is_empty() {
            return None;
        }

        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        let mut changed = false;
        for (note, old_key) in &originals {
            let new_note = humanize_note(*note, params, rng);
            if new_note.start_tick != note.start_tick
                || new_note.end_tick != note.end_tick
                || new_note.velocity != note.velocity
            {
                changed = true;
            }
            new_by_key.entry(*old_key).or_default().push(new_note);
        }
        if !changed {
            // 原样插回，模型内容不变（无 undo 动作）
            batch_ops::insert_batch(model, new_by_key);
            model.rebuild_dirty();
            return None;
        }

        let after = batch_ops::flatten_by_key(&new_by_key);
        batch_ops::insert_batch(model, new_by_key);
        model.rebuild_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: originals,
            after,
        }))
    }
}

/// xorshift64 伪随机：人性化抖动足够（无需密码学质量）。
fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// 闭区间 `[-amp, amp]` 的随机整数（含 0）；`amp == 0` 时恒为 0（不消耗随机数）。
fn rand_offset(rng: &mut u64, amp: u32) -> i64 {
    if amp == 0 {
        return 0;
    }
    let span = amp as u64 * 2 + 1;
    (xorshift64(rng) % span) as i64 - amp as i64
}

/// 对单个音符施加三路随机偏差（见 [`Document::humanize_selected`]）。
fn humanize_note(
    note: yinhe_types::Note,
    params: HumanizeParams,
    rng: &mut u64,
) -> yinhe_types::Note {
    let mut n = note;
    // 时间：整体平移，gate 不变；start 不越过 0、end 永远大于 start。
    let dt = rand_offset(rng, params.timing);
    if dt != 0 {
        let s = (n.start_tick as i64 + dt).clamp(0, u32::MAX as i64 - 1);
        let e = (n.end_tick as i64 + dt).clamp(s + 1, u32::MAX as i64);
        n.start_tick = s as u32;
        n.end_tick = e as u32;
    }
    // 力度：clamp 0..=127。
    let dv = rand_offset(rng, params.velocity as u32);
    if dv != 0 {
        n.velocity = (n.velocity as i64 + dv).clamp(0, 127) as u8;
    }
    // 时值：只动 end（gate 最短 1 tick）。
    let dg = rand_offset(rng, params.gate);
    if dg != 0 {
        let len = (n.end_tick - n.start_tick) as i64 + dg;
        let new_end = n.start_tick as i64 + len.max(1);
        n.end_tick = new_end.clamp(n.start_tick as i64 + 1, u32::MAX as i64) as u32;
    }
    n
}
