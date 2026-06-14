use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum StageStatus {
    Pending,
    Active,
    Done,
}

#[derive(Clone)]
pub(crate) struct StageInfo {
    pub label: String,
    pub progress: f32,
    pub status: StageStatus,
    pub detail: String,
}

#[derive(Clone)]
pub(crate) struct LoadProgress {
    pub stages: Vec<StageInfo>,
    pub visible: bool,
}

pub(crate) type SharedProgress = Arc<Mutex<LoadProgress>>;

pub(crate) fn new_shared() -> SharedProgress {
    Arc::new(Mutex::new(LoadProgress {
        stages: vec![
            StageInfo {
                label: "解析 MIDI 音轨".into(),
                progress: 0.0,
                status: StageStatus::Pending,
                detail: String::new(),
            },
            StageInfo {
                label: "转换存档格式".into(),
                progress: 0.0,
                status: StageStatus::Pending,
                detail: String::new(),
            },
            StageInfo {
                label: "初始化音频引擎".into(),
                progress: 0.0,
                status: StageStatus::Pending,
                detail: String::new(),
            },
            StageInfo {
                label: "加载音色库".into(),
                progress: 0.0,
                status: StageStatus::Pending,
                detail: String::new(),
            },
        ],
        visible: false,
    }))
}

pub(crate) fn set_stage(progress: &SharedProgress, idx: usize, status: StageStatus) {
    if let Ok(mut p) = progress.lock() {
        if idx < p.stages.len() {
            p.stages[idx].status = status;
        }
    }
}

pub(crate) fn set_stage_progress(
    progress: &SharedProgress,
    idx: usize,
    pct: f32,
    detail: String,
) {
    if let Ok(mut p) = progress.lock() {
        if idx < p.stages.len() {
            p.stages[idx].progress = pct;
            p.stages[idx].detail = detail;
            p.stages[idx].status = StageStatus::Active;
        }
    }
}

pub(crate) fn set_visible(progress: &SharedProgress, visible: bool) {
    if let Ok(mut p) = progress.lock() {
        p.visible = visible;
    }
}
