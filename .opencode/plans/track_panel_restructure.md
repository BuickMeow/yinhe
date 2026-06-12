# 音轨走带面板重构计划

## 概览

将 arrangement 视图左侧的 track panel 从"逐轨平铺排列"改造为"按通道分组的文件夹结构"，并在每个通道下显示 CC 自动化子行。同时新增 Conductor 轨显示 BPM 自动化。

---

## 一、数据模型变更

### 1.1 新增 `AutomationTarget::Tempo` 变体

**文件**: `crates/yinhe-types/src/automation.rs`

在 `AutomationTarget` 枚举中新增：
```rust
/// BPM / Tempo automation (from conductor track tempo_segments).
Tempo,
```

需要更新的方法：
- `max_value()` → 300（BPM 上限）
- `default_value()` → 120
- `has_center_line()` → false
- `display_name()` → "Tempo (BPM)"

**影响范围**: 所有 `match` on `AutomationTarget` 的地方需要新增 arm。涉及：
- `crates/yinhe-types/src/automation.rs`（测试）
- `crates/yinhe-automation/src/automation_instances.rs`（渲染逻辑，Tempo 无特殊处理即可）
- `crates/yinhe-egui/src/automation_panel.rs`（下拉菜单，可选择性加入 Tempo 选项）

### 1.2 新增 `build_tempo_automation_lane` 函数

**文件**: `crates/yinhe-midi/src/midi.rs`

```rust
pub fn build_tempo_automation_lane(
    tempo_segments: &[TempoSegment],
    _ticks_per_beat: u32,
) -> AutomationLane {
    // 将 TempoSegment 转换为 AutomationLane
    // value = (bpm * 100) as u16（取整保留精度，max=30000）
    // 每个 segment 产生一个 AutomationEvent
}
```

导出至 `crates/yinhe-midi/src/lib.rs`。

### 1.3 通道分组数据结构

**文件**: `crates/yinhe-egui/src/widgets/track_panel.rs`

```rust
/// 一个通道组（Port+Channel 组合）
pub(crate) struct ChannelGroup {
    /// 端口号 (0-15)
    pub port: u8,
    /// MIDI 通道号 (1-16)
    pub channel: u8,
    /// 属于此通道的 track 索引列表（排序后）
    pub track_indices: Vec<u16>,
    /// 文件夹是否展开
    pub expanded: bool,
    /// 每个 CC 控制器的子行（从 automation_lanes 构建）
    pub cc_lanes: Vec<CcLane>,
}

/// CC 自动化子行
pub(crate) struct CcLane {
    /// CC 控制器号
    pub controller: u8,
    /// 显示名称（如 "Volume", "Pan"）
    pub name: String,
    /// 是否展开
    pub expanded: bool,
}
```

---

## 二、Document 状态变更

**文件**: `crates/yinhe-egui/src/document.rs`

新增字段：
```rust
pub struct Document {
    // ... existing fields ...

    /// 通道分组（按 port+channel 组合，从 track_info_cache 构建）
    pub channel_groups: Vec<ChannelGroup>,

    /// Conductor 轨是否展开
    pub conductor_expanded: bool,

    /// BPM 自动化 lane（从 tempo_segments 构建）
    pub tempo_lane: Option<yinhe_types::AutomationLane>,
}
```

在 `Document::from_midi()` 和 `Document::from_yin()` 中：
1. 调用 `build_tempo_automation_lane()` 构建 `tempo_lane`
2. 从 `track_info_cache` 按 port+channel 分组构建 `channel_groups`
3. 初始化 `conductor_expanded = true`，每个 group `expanded = true`

在 `Document::empty()` 中：
1. `channel_groups = vec![]`
2. `conductor_expanded = true`
3. `tempo_lane = None`

---

## 三、Track Panel 重写（核心）

**文件**: `crates/yinhe-egui/src/widgets/track_panel.rs`

### 3.1 行类型枚举

```rust
enum TrackPanelRow {
    /// Conductor 轨（固定在顶部）
    Conductor,
    /// BPM 自动化子行（在 Conductor 展开时显示）
    BpmAutomation,
    /// 通道文件夹头
    FolderHeader(usize),       // group index
    /// 通道内的 track 行
    Track(usize, usize),       // (group index, track local index)
    /// CC 自动化子行
    CcAutomation(usize, usize), // (group index, cc lane index)
}
```

### 3.2 行布局计算

不再使用 `idx * row_height`，改为遍历所有可见行并累计高度：

```rust
struct RowLayout {
    rows: Vec<(TrackPanelRow, f32)>,  // (行类型, 行高)
    total_height: f32,
}

fn compute_rows(
    channel_groups: &[ChannelGroup],
    conductor_expanded: bool,
    tempo_lane: &Option<AutomationLane>,
    base_row_height: f32,
) -> RowLayout {
    // Conductor 行: 24px
    // FolderHeader 行: 24px
    // Track 行: base_row_height
    // CcAutomation 行: 24px
    // BpmAutomation 行: 24px
}
```

### 3.3 虚拟滚动适配

行高可变后，虚拟滚动从"等高行"改为"累计高度前缀和 + 二分查找"：

```rust
// 计算可见行范围
fn visible_rows(total_height: f32, row_heights: &[f32], scroll_y: f32, panel_h: f32) -> (usize, usize) {
    // 前缀和 + partition_point 找到 first 可见行
    // 继续向后直到超出 panel_h
}
```

### 3.4 渲染逻辑

`show()` 函数内部改为：

1. **计算行布局** `compute_rows()`
2. **虚拟滚动**：基于累计高度确定 first/last 可见行
3. **逐行渲染**：
   - **Conductor 行**：带 ▶/▼ 折叠图标 + "Conductor" 标签 + 蓝色条
   - **BpmAutomation 行**：缩进显示，"BPM" 标签 + 小型 automation 条预览（从 `tempo_lane` 渲染）
   - **Folder Header**：带 ▶/▼ 折叠图标 + 通道标签（如 "A01"）+ 颜色条
   - **Track 行**：缩进显示，保留现有 track number + badge + note count + PC + name
   - **CcAutomation 行**：缩进显示，CC 名称标签 + 小型 automation 条预览
4. **交互**：
   - 点击 Folder Header → 切换 `expanded`
   - 点击 Conductor → 切换 `conductor_expanded`
   - 点击 Track 行 → 选中 track（保持 `track_selected` 逻辑）
   - 滚轮 → 基于累计高度的 scroll

### 3.5 签名变更

```rust
pub(crate) fn show(
    ui: &mut egui::Ui,
    track_info: &[TrackInfo],
    track_visible: &mut [bool],
    track_selected: &mut Option<u16>,
    pc_map: &HashMap<u8, u8>,
    row_height: &mut f32,
    scroll_y: &mut f32,
    // 新增参数：
    channel_groups: &mut [ChannelGroup],
    conductor_expanded: &mut bool,
    tempo_lane: &Option<AutomationLane>,
)
```

---

## 四、调用方更新

### 4.1 `arrange.rs` (行 117-125)

更新 `track_panel::show()` 调用：
```rust
track_panel::show(
    ui,
    &doc.track_info_cache,
    &mut doc.track_visible,
    &mut doc.track_selected,
    &doc.pc_map_cache,
    &mut arr_view.base.track_panel_row_height,
    &mut arr_view.base.track_panel_scroll_y,
    &mut doc.channel_groups,
    &mut doc.conductor_expanded,
    &doc.tempo_lane,
);
```

### 4.2 Pianoroll 视图清理

**文件**: `crates/yinhe-egui/src/piano_view.rs`

确认无残留的 track panel 调用代码。当前 pianoroll 视图已没有 track panel，如有残留则删除。

---

## 五、Arrangement GPU 视图（本阶段不变）

当前 arrangement GPU 渲染（`yinhe-arrangement/src/instances.rs`）假设每个 track lane 等高（`lane_height`）。**本阶段不做 GPU 视图同步**——track panel 是独立的左侧信息面板，右侧 GPU 渲染保持现有 flat track 布局。

两者通过 `track_selected` 关联，但行位置不需要严格对齐。后续如需对齐，可将 `ArrangementView` 也改为 hierarchical layout。

---

## 六、实施步骤

| Step | 内容 | 文件 |
|------|------|------|
| 1 | 新增 `AutomationTarget::Tempo` + `build_tempo_automation_lane()` | `yinhe-types/src/automation.rs`, `yinhe-midi/src/midi.rs`, `yinhe-midi/src/lib.rs` |
| 2 | 定义 `ChannelGroup`, `CcLane` 数据结构 | `yinhe-egui/src/widgets/track_panel.rs` |
| 3 | 在 Document 中新增字段并初始化 | `yinhe-egui/src/document.rs` |
| 4 | 重写 `track_panel::show()` | `yinhe-egui/src/widgets/track_panel.rs` |
| 5 | 更新 `arrange.rs` 调用 | `yinhe-egui/src/arrange.rs` |
| 6 | 清理 pianoroll 残留（如有） | `yinhe-egui/src/piano_view.rs` |
| 7 | `cargo build` + `cargo test` | — |

---

## 七、文件变更清单

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `crates/yinhe-types/src/automation.rs` | 修改 | 新增 `Tempo` 变体 |
| `crates/yinhe-midi/src/midi.rs` | 修改 | 新增 `build_tempo_automation_lane()` |
| `crates/yinhe-midi/src/lib.rs` | 修改 | 导出新函数 |
| `crates/yinhe-egui/src/document.rs` | 修改 | 新增 channel_groups, conductor_expanded, tempo_lane |
| `crates/yinhe-egui/src/widgets/track_panel.rs` | **重写** | 通道分组 + 文件夹 + 自动化子行 |
| `crates/yinhe-egui/src/arrange.rs` | 修改 | 更新 track_panel::show() 调用 |
| `crates/yinhe-egui/src/piano_view.rs` | 可能修改 | 清理残留 |

---

## 八、风险与注意事项

1. **Arrangement GPU 不对齐**：track panel 行高变为可变，与右侧 GPU lanes 不再严格对齐。这是可接受的（DAW 常见模式），后续可对齐。

2. **AutomationTarget::Tempo 编译影响**：新增枚举变体会导致所有 exhaustive match 产生编译错误，需逐一添加 arm。

3. **性能**：`compute_rows()` 每帧调用，但行数有限（通道数 × (1 + CC 数)），不会成为瓶颈。

4. **虚拟滚动**：行高可变后需要前缀和 + 二分查找，逻辑稍复杂但直接。
