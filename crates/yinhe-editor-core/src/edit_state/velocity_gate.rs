use super::EditState;

impl EditState {
    /// 新建音符的默认力度：该轨最近一次修改值，无记录回退 100
    pub fn default_velocity(&self, track: u16) -> u8 {
        self.recent_velocity
            .get(track as usize)
            .and_then(|v| *v)
            .map(|(_, v)| v)
            .unwrap_or(100)
    }

    /// 记录一次 velocity 修改，保留 start_tick 最晚的值
    pub fn remember_velocity(&mut self, track: u16, start_tick: u32, velocity: u8) {
        let i = track as usize;
        if self.recent_velocity.len() <= i {
            self.recent_velocity.resize(i + 1, None);
        }
        let slot = &mut self.recent_velocity[i];
        if slot.is_none_or(|(t, _)| start_tick >= t) {
            *slot = Some((start_tick, velocity));
        }
    }

    /// 新建音符的默认长度，无记录回退 fallback
    pub fn default_gate(&self, track: u16, fallback: u32) -> u32 {
        self.recent_gate
            .get(track as usize)
            .and_then(|v| *v)
            .map(|(_, g)| g)
            .unwrap_or(fallback)
    }

    /// 记录一次 gate 修改，保留 start_tick 最晚的值
    pub fn remember_gate(&mut self, track: u16, start_tick: u32, gate: u32) {
        let i = track as usize;
        if self.recent_gate.len() <= i {
            self.recent_gate.resize(i + 1, None);
        }
        let slot = &mut self.recent_gate[i];
        if slot.is_none_or(|(t, _)| start_tick >= t) {
            *slot = Some((start_tick, gate));
        }
    }
}
