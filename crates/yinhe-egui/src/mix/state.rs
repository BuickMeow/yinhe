use super::*;

/// 单个电平表的 UI 侧弹道状态（L/R 各一路：当前电平 + 峰值保持）。
///
/// 起跳瞬时跟随原始峰值，回落在 **dB 域**匀速（线性域递减在接近 0 时会显得忽快忽慢，
/// 非行业惯用）。峰值保持线先停留 `METER_HOLD_SECS`，之后同样按 dB 速度回落。
#[derive(Clone, Copy, Default)]
pub(crate) struct MeterState {
    pub(crate) level: [f32; 2],
    pub(crate) hold: [f32; 2],
    pub(crate) hold_age: [f32; 2],
}

/// MIX 界面的非持久化 UI 状态。
#[derive(Default)]
pub(crate) struct MixUiState {
    /// 各 dense 通道的电平表弹道状态，UI 侧衰减用。
    pub(crate) smoothed: Vec<MeterState>,
    pub(crate) smoothed_master: MeterState,
    /// 本帧总输出电平显示值 `((level L,R), (hold L,R))`：main_loop 每帧只更新
    /// 一次（[`update_master_meter`]），混音台 master 条与走带栏时间码表共用，
    /// 避免同一帧两次衰减。
    pub(crate) master_display: ((f32, f32), (f32, f32)),
    /// 每轨电平表弹道状态（索引 = 音轨；仅播放时有值）。
    pub(crate) smoothed_tracks: Vec<MeterState>,
    /// 本帧每轨电平表显示值 `((L,R), (holdL,holdR))`，main_loop 每帧更新
    ///（仅播放时非零）。
    pub(crate) track_display: Vec<crate::widgets::meter::StereoMeter>,
    /// 插件扫描结果（后台线程填充；None = 尚未完成首次扫描）。
    pub(crate) scanned: Option<Vec<PluginEntry>>,
    /// 扫描中失败的包数量（诊断展示）。
    pub(crate) scan_errors: usize,
    /// 扫描 worker 进度接收端（Some = 扫描进行中）。
    pub(crate) scan_rx: Option<std::sync::mpsc::Receiver<ScanProgress>>,
    /// 后台扫描进行中（UI 状态展示用）。
    pub(crate) scan_in_progress: bool,
    /// 插件选择器打开目标（None = 关闭）。
    pub(crate) picker_for: Option<InsertTarget>,
    /// 当前 SEND 聚焦目标；其他通道显示到该通道的 dB 旋钮。
    pub(crate) send_focus: Option<MixerChannelId>,
    /// 本帧 CH 显示编号（ChannelId -> CH n）。
    pub(crate) channel_display: std::collections::HashMap<ChannelId, u32>,
    /// 发送面板打开目标（None = 关闭）。
    /// XSynth 配置窗口目标（源通道；None = 关闭）。
    pub(crate) synth_config_for: Option<u8>,
    /// 乐器选择器目标：MIDI 通道（挂载点）；None = 未打开。
    pub(crate) instrument_picker_for: Option<u8>,
    pub(crate) picker_filter: String,
    /// 插件选择器左侧分类页签（打开时按语境初始化）。
    pub(crate) picker_tab: strip::PickerTab,
    /// 插件选择器当前选中条目 id（`PluginEntry.id`；内置 XSynth 用
    /// [`strip::XS_SELECTED_ID`]）。
    pub(crate) picker_selected: Option<String>,
    /// 插件选择器星标筛选（点亮 = 只显示已收藏插件）。
    pub(crate) picker_fav_only: bool,
    /// 插件参数面板（同一时间至多一个）。
    pub(crate) param_panel: Option<ParamPanel>,
    /// 内置音源参数窗口（同一时间至多一个）。
    pub(crate) synth_params_panel: Option<crate::dialogs::instrument_params::InstrumentParamsPanel>,
}

pub(crate) const SOURCE_CHANNELS: usize = yinhe_mixer::CHANNEL_COUNT;
pub(crate) const METER_DB_PER_SEC: f32 = 20.0;
pub(crate) const METER_HOLD_SECS: f32 = 1.0;

impl MeterState {
    pub(crate) fn update(&mut self, raw: (f32, f32), dt: f32) {
        let drop_db = METER_DB_PER_SEC * dt;
        let raw = [raw.0, raw.1];
        for (i, &r) in raw.iter().enumerate() {
            let prev_db = gain_to_db(self.level[i]);
            let raw_db = gain_to_db(r);
            let new_db = if raw_db >= prev_db {
                raw_db
            } else {
                (prev_db - drop_db).max(raw_db)
            };
            self.level[i] = db_to_gain(new_db);
            if r >= self.hold[i] {
                self.hold[i] = r;
                self.hold_age[i] = 0.0;
            } else {
                self.hold_age[i] += dt;
                if self.hold_age[i] > METER_HOLD_SECS {
                    self.hold[i] = db_to_gain(
                        (gain_to_db(self.hold[i]) - drop_db).max(gain_to_db(self.level[i])),
                    );
                }
            }
        }
    }

    pub(crate) fn level(&self) -> (f32, f32) {
        (self.level[0], self.level[1])
    }

    pub(crate) fn hold(&self) -> (f32, f32) {
        (self.hold[0], self.hold[1])
    }
}
