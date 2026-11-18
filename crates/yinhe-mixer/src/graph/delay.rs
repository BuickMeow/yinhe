pub(crate) struct DelayLine {
    left: Vec<f32>,
    right: Vec<f32>,
    write: usize,
}

impl DelayLine {
    pub(crate) fn new() -> Self {
        Self {
            left: Vec::new(),
            right: Vec::new(),
            write: 0,
        }
    }

    pub(crate) fn add_delayed_into(
        &mut self,
        src_l: &[f32],
        src_r: &[f32],
        dst_l: &mut [f32],
        dst_r: &mut [f32],
        amount: f32,
        frames: usize,
    ) {
        if self.left.is_empty() {
            for i in 0..frames {
                dst_l[i] += src_l[i] * amount;
                dst_r[i] += src_r[i] * amount;
            }
            return;
        }
        for i in 0..frames {
            let idx = (self.write + i) % self.left.len();
            dst_l[i] += self.left[idx] * amount;
            dst_r[i] += self.right[idx] * amount;
            self.left[idx] = src_l[i];
            self.right[idx] = src_r[i];
        }
        self.write = (self.write + frames) % self.left.len();
    }

    #[allow(dead_code)] // 仅测试/诊断读取
    pub(crate) fn delay(&self) -> usize {
        self.left.len()
    }

    /// 设定延迟样本数（变化时重建缓冲；仅命令处理阶段调用，允许分配）。
    pub(crate) fn set_delay(&mut self, delay: usize) {
        if self.left.len() != delay {
            self.left = vec![0.0; delay];
            self.right = vec![0.0; delay];
            self.write = 0;
        }
    }

    pub(crate) fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.left.is_empty() {
            return;
        }
        let len = self.left.len();
        for i in 0..left.len() {
            let out_l = self.left[self.write];
            let out_r = self.right[self.write];
            self.left[self.write] = left[i];
            self.right[self.write] = right[i];
            left[i] = out_l;
            right[i] = out_r;
            self.write += 1;
            if self.write == len {
                self.write = 0;
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.write = 0;
    }
}
