use yinhe_types::MAX_KEY;

/// 选框拖拽时拖动哪条边
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeSide {
    Left,
    Right,
}

/// 选框矩形状态：可视选框的单一真相来源
/// 替代原先分散在 egui 持久化中的 sel_rect_persist 等
#[derive(Clone, Default)]
pub struct SelRectState {
    /// 已提交的选框：(t_start, t_end, key_lo, key_hi)，shift+框选时追加
    pub rects: Vec<(f64, f64, u8, u8)>,
    /// 与 rects 平行的标记：是否为框选空白区自动生成的全键选框
    pub auto_vertical: Vec<bool>,
    drag_origins: Vec<(f64, f64, u8, u8)>,
    drag_delta: Option<(i64, i32)>,
    resize_origins: Vec<(f64, f64, u8, u8)>,
    resize_side: Option<ResizeSide>,
    resize_dt: Option<i64>,
}

impl SelRectState {
    fn offset_rect(rect: (f64, f64, u8, u8), dt: i64, dk: i32) -> (f64, f64, u8, u8) {
        let (t0, t1, kl, kh) = rect;
        (
            t0 + dt as f64,
            t1 + dt as f64,
            (kl as i32 + dk).clamp(0, MAX_KEY as i32) as u8,
            (kh as i32 + dk).clamp(0, MAX_KEY as i32) as u8,
        )
    }

    fn resize_rect(rect: (f64, f64, u8, u8), side: ResizeSide, dt: i64) -> (f64, f64, u8, u8) {
        let (t0, t1, kl, kh) = rect;
        match side {
            ResizeSide::Left => {
                let new_t0 = (t0 + dt as f64).max(0.0).min(t1 - 1.0);
                (new_t0, t1, kl, kh)
            }
            ResizeSide::Right => {
                let new_t1 = (t1 + dt as f64).max(t0 + 1.0);
                (t0, new_t1, kl, kh)
            }
        }
    }

    /// 有效选框：拖拽/缩放时返回 origins+delta，否则返回 rects
    pub fn effective_rects(&self) -> Vec<(f64, f64, u8, u8)> {
        if let Some((dt, dk)) = self.drag_delta {
            self.drag_origins
                .iter()
                .map(|&r| Self::offset_rect(r, dt, dk))
                .collect()
        } else if let (Some(side), Some(dt)) = (self.resize_side, self.resize_dt) {
            self.resize_origins
                .iter()
                .map(|&r| Self::resize_rect(r, side, dt))
                .collect()
        } else {
            self.rects.clone()
        }
    }

    pub fn is_resizing(&self) -> bool {
        self.resize_side.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
        self.auto_vertical.clear();
    }

    pub fn push_rect(&mut self, rect: (f64, f64, u8, u8), auto_vertical: bool) {
        self.rects.push(rect);
        self.auto_vertical.push(auto_vertical);
    }

    pub fn has_auto_vertical(&self) -> bool {
        self.auto_vertical.iter().any(|&b| b)
    }

    pub fn start_drag(&mut self) {
        self.drag_origins = self.rects.clone();
        self.drag_delta = None;
    }

    pub fn update_drag(&mut self, dt: i64, dk: i32) {
        self.drag_delta = Some((dt, dk));
    }

    pub fn end_drag(&mut self) {
        if let Some((dt, dk)) = self.drag_delta {
            self.rects = self
                .drag_origins
                .iter()
                .map(|&r| Self::offset_rect(r, dt, dk))
                .collect();
        }
        self.drag_origins.clear();
        self.drag_delta = None;
    }

    pub fn cancel_drag(&mut self) {
        self.drag_origins.clear();
        self.drag_delta = None;
    }

    pub fn start_resize(&mut self, side: ResizeSide) {
        self.resize_origins = self.rects.clone();
        self.resize_side = Some(side);
        self.resize_dt = None;
    }

    pub fn update_resize(&mut self, dt: i64) {
        self.resize_dt = Some(dt);
    }

    pub fn end_resize(&mut self) {
        if let (Some(side), Some(dt)) = (self.resize_side, self.resize_dt) {
            self.rects = self
                .resize_origins
                .iter()
                .map(|&r| Self::resize_rect(r, side, dt))
                .collect();
        }
        self.resize_origins.clear();
        self.resize_side = None;
        self.resize_dt = None;
    }

    pub fn cancel_resize(&mut self) {
        self.resize_origins.clear();
        self.resize_side = None;
        self.resize_dt = None;
    }
}
