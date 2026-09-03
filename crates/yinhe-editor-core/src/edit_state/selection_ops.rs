use yinhe_types::MAX_KEY;

use super::EditState;

impl EditState {
    /// 音轨结构变化后重映射所有 `(track_idx, target)` 键
    pub fn remap_am_track_keys(&mut self, remap: impl Fn(u16) -> Option<u16> + Copy) {
        let remap_entry =
            |(t, target): (u16, yinhe_types::AutomationTarget)| remap(t).map(|nt| (nt, target));
        self.arr_am_ms = std::mem::take(&mut self.arr_am_ms)
            .into_iter()
            .filter_map(|(k, v)| remap_entry(k).map(|nk| (nk, v)))
            .collect();
        self.arr_am_views = std::mem::take(&mut self.arr_am_views)
            .into_iter()
            .filter_map(|(k, v)| remap_entry(k).map(|nk| (nk, v)))
            .collect();
        self.arr_am_selected = std::mem::take(&mut self.arr_am_selected)
            .into_iter()
            .filter_map(remap_entry)
            .collect();
    }

    /// 音符选框整体 tick 平移
    pub fn offset_sel_ticks(&mut self, dt: i64) {
        self.selected.offset_ticks(dt);
        for r in &mut self.sel_rect.rects {
            r.0 += dt as f64;
            r.1 += dt as f64;
        }
        for r in &mut self.arr_sel_rect {
            r.0 += dt as f64;
            r.1 += dt as f64;
        }
    }

    /// 音符选框整体 key 平移，同步更新 auto_vertical 标记
    pub fn offset_sel_keys(&mut self, dk: i32) {
        self.selected.offset(0, dk);
        let still_vertical: Vec<bool> = self
            .sel_rect
            .rects
            .iter()
            .zip(&self.sel_rect.auto_vertical)
            .map(|(r, &auto)| {
                let kl = (r.2 as i32 + dk).clamp(0, MAX_KEY as i32) as u8;
                let kh = (r.3 as i32 + dk).clamp(0, MAX_KEY as i32) as u8;
                auto && kl == 0 && kh == MAX_KEY
            })
            .collect();
        for r in &mut self.sel_rect.rects {
            r.2 = (r.2 as i32 + dk).clamp(0, MAX_KEY as i32) as u8;
            r.3 = (r.3 as i32 + dk).clamp(0, MAX_KEY as i32) as u8;
        }
        self.sel_rect.auto_vertical = still_vertical;
    }

    /// 音符选框 tick 终点统一平移（gate 加减用，起点不动）
    pub fn offset_sel_te(&mut self, dt: i64) {
        for r in &mut self.sel_rect.rects {
            r.1 = (r.1 + dt as f64).max(r.0 + 1.0);
        }
        for r in &mut self.arr_sel_rect {
            r.1 = (r.1 + dt as f64).max(r.0 + 1.0);
        }
        for r in &mut self.selected.rects {
            let new_te = (r.1 as i64 + dt).max(r.0 as i64 + 1) as u32;
            r.1 = new_te;
        }
    }

    /// 音符选框 tick 范围相对 t0 等比缩放
    pub fn scale_sel_ticks(&mut self, t0: u64, factor: f64) {
        let scale = |v: u64| -> u64 {
            let s = (t0 as f64 + (v as f64 - t0 as f64) * factor)
                .round()
                .max(t0 as f64);
            if s > u32::MAX as f64 {
                u32::MAX as u64
            } else {
                s as u64
            }
        };
        let scale_rect = |ts: &mut u64, te: &mut u64| {
            let nts = scale(*ts);
            let nte = scale(*te).max(nts + 1);
            *ts = nts;
            *te = nte;
        };
        for r in &mut self.selected.rects {
            let mut ts = r.0 as u64;
            let mut te = r.1 as u64;
            scale_rect(&mut ts, &mut te);
            r.0 = ts as u32;
            r.1 = te as u32;
        }
        for r in &mut self.sel_rect.rects {
            let mut ts = r.0 as u64;
            let mut te = r.1 as u64;
            scale_rect(&mut ts, &mut te);
            r.0 = ts as f64;
            r.1 = te as f64;
        }
        for r in &mut self.arr_sel_rect {
            let mut ts = r.0 as u64;
            let mut te = r.1 as u64;
            scale_rect(&mut ts, &mut te);
            r.0 = ts as f64;
            r.1 = te as f64;
        }
    }

    /// AM 选框 tick 范围平移
    pub fn offset_anchor_ticks(&mut self, panel_idx: usize, dt: i64) {
        if let Some(panel) = self.controller_panels.get_mut(panel_idx) {
            for r in &mut panel.anchor_sel_rects {
                r.tick_start += dt as f64;
                r.tick_end += dt as f64;
            }
        }
    }

    /// AM 选框 value 范围平移
    pub fn offset_anchor_values(&mut self, panel_idx: usize, dv: f32) {
        if let Some(panel) = self.controller_panels.get_mut(panel_idx) {
            for r in &mut panel.anchor_sel_rects {
                if let Some((lo, hi)) = &mut r.value_range {
                    *lo += dv;
                    *hi += dv;
                }
            }
        }
    }

    /// AM 选框 tick 范围相对 t0 等比缩放
    pub fn scale_anchor_ticks(&mut self, panel_idx: usize, t0: f64, factor: f64) {
        if let Some(panel) = self.controller_panels.get_mut(panel_idx) {
            for r in &mut panel.anchor_sel_rects {
                let ts = r.tick_start.min(r.tick_end);
                let te = r.tick_start.max(r.tick_end);
                let nts = (t0 + (ts - t0) * factor).round();
                let nte = (t0 + (te - t0) * factor).round().max(nts + 1.0);
                r.tick_start = nts;
                r.tick_end = nte;
            }
        }
    }
}
