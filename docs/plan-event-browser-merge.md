# Event Browser 合并方案

## 现状问题

当前两个 tab 内容完全不同，且数据割裂：

| 视图 | 树结构 | 事件详情 | 实时刷新 |
|------|--------|----------|----------|
| 实时事件 | 扁平轨道列表 | ✅ 表格展示 | ❌ fingerprint 只比较数量，不追踪内容变更 |
| 归档结构 | Port → Channel → Track 树 | ❌ 只有元数据和计数 | ❌ 同上 |

用户需要在两个 tab 之间来回切换才能完成"找到某条轨道 → 查看具体事件"这一基本操作。

## 目标

**合并为一个视图**，以归档树为骨架，嵌入实时事件详情能力：

```
Port A
├── Channel 1
│   ├── Track "Piano"
│   │   ├── Notes (342)       ← 可展开为表格
│   │   ├── CC 7 Volume (12)  ← 可展开为表格
│   │   └── Pitch Bend (5)    ← 可展开为表格
│   └── Track "Bass"
│       └── Notes (128)
└── Channel 2
    └── Conductor              ← Tempo / TimeSig
        ├── Tempo (3)
        └── TimeSig (2)
```

## 具体改动

### 1. 数据结构合并

移除 `ViewTab` 枚举，`EventBrowserState` 合并为单一状态：

```rust
pub struct EventBrowserState {
    // 树展开状态 — 沿用 ArchiveKey
    pub expanded_keys: HashSet<ArchiveKey>,
    // 选中项 — 统一用 SelectedItem（已有，覆盖所有事件类型）
    pub selected_item: Option<SelectedItem>,
    // 选中轨道 — 用于底部详情面板的元数据展示
    pub selected_track: Option<u16>,
    // 指纹 — 改用内容哈希，追踪真实变更
    fingerprint: Option<u64>,
    // 分割比例
    split_ratio: f32,
}
```

### 2. 树结构统一

- **轨道按 port → channel 分组**（沿用 `group_tracks_by_port_channel`）
- **Conductor 放在树根**（Port 0 / Channel 0 之外，或作为特殊节点）
- **每个轨道叶子可展开**，显示 Notes / CC / PitchBend / ProgramChange 子节点
- **点击事件叶子** → 底部面板显示事件表格（同当前实时视图的 `show_realtime_detail`）

### 3. 底部详情面板

- 选中事件叶子 → 显示事件表格（当前 `show_realtime_detail` 逻辑）
- 选中轨道（点击轨道名而非事件） → 显示轨道元数据（当前 `show_archive_track_detail` 逻辑）
- 无选中 → 显示概览（当前 `show_archive_overview` 逻辑）

### 4. 实时刷新

当前 fingerprint 实现：

```rust
// 实时视图
let fingerprint = model.tick_length
    ^ (model.note_count << 16)
    ^ (model.tracks.len() as u64).wrapping_mul(0x9E3779B9);

// 归档视图
let fingerprint = (model.tracks.len() as u64)
    ^ (model.note_count << 16)
    ^ model.tick_length.wrapping_mul(0x9E3779B9);
```

两个都**不追踪内容变更**——修改一个音符的 tick 不会改变 fingerprint，因为 `note_count` 没变。

改为使用 `model.midi_version`（`ProjectData::midi_version`，每次 `bump_version()` 递增）：

```rust
let fingerprint = doc.data.midi_version;
```

每次编辑操作（添加/删除/修改音符、CC 等）都会调用 `bump_version()`，fingerprint 变化触发树状态刷新。

### 5. 状态保留策略

fingerprint 变化时：
- 保留 `expanded_keys`（不折叠已展开的节点）
- 保留 `selected_item`（如果对应的 track/controller 仍然存在）
- 清除 `selected_track`（如果 track index 越界）

### 6. 删除内容

- 删除 `ViewTab` 枚举
- 删除 `active_tab` 字段
- 删除 `expanded_tracks`（改用 `expanded_keys` 统一管理）
- 删除 `expanded_archive_keys`（改名为 `expanded_keys`）
- 删除 `archive_fingerprint`（合并为单一 `fingerprint`）
- 删除 `selected_archive_track`（合并为 `selected_track`）
- 删除 `show_realtime` / `show_archive` 两个入口函数
- 删除 `show_realtime_overview`（保留 `show_archive_overview` 作为概览）
- 删除 `render_realtime_tree`（树渲染统一为 `render_archive_tree` 的增强版）

### 7. 文件结构

当前 `event_browser.rs` 1008 行，合并后预计 1100-1200 行。暂不拆分，等后续重构。

## 实施步骤

1. 修改 `EventBrowserState`：合并字段，删除 `ViewTab`
2. 修改 `show()`：去掉 tab bar，直接显示统一视图
3. 增强 `render_archive_tree`：轨道叶子下增加事件子节点（Notes/CC/PitchBend/ProgramChange）
4. 底部面板：根据选中类型分派到 `show_realtime_detail` 或 `show_archive_track_detail`
5. 修改 fingerprint：使用 `midi_version`
6. 清理：删除废弃的实时视图函数和字段
7. 测试：更新现有测试，补充合并后的行为测试
