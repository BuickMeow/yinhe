# yinhe-egui 待办清单

## Bug 修复（高优先级）

- [ ] **视口不刷新** — `pianoroll_prepare.rs:132` 的 `notes_key` 不含 MIDI 内容哈希。Document 加 `midi_version: u64`，`make_mut`/`restore` 后 +1，key 哈希 version
- [ ] **event browser 不追踪变更** — `event_browser.rs:174` fingerprint 只比较条目数量。改为内容哈希或 `doc.midi.note_count`
- [ ] **apply_snapshot 不同步缓存** — `app_actions.rs:260` 只同步 `ti.name`。改为 `doc.track_info_cache = doc.midi.track_info()` + 重建 `pc_map_cache`

## undo/redo 性能（千万音符）

- [ ] 增量 `note_count` / `max_tick` — 编辑时直接 `+=1` / `-=1`，不全量遍历
- [ ] 增量 `NoteScanIndex` — 只重建受影响 key 的 blocks
- [ ] 增量 `build_automation_lanes` — 只重建受影响 track

## Document 拆解（渐进式，每步 cargo check）

- [ ] **Phase 1 facade** — 引入 `DocData` / `DocEdit` / `DocCache` / `DocIdentity` sub-structs，Document 保持 delegation methods（`doc.midi()` / `doc.selected()` 等），不改任何调用点
- [ ] **Phase 2 推参数** — 从叶子函数（right_panel/）开始，`&mut Document` → sub-struct refs。每改一个模块 cargo check
- [ ] **Phase 3 精简 Snapshot** — 只存 `DocData`，不存整个 Document

## 单元测试

- [ ] `history.rs` — push/undo/redo 基本流、MAX_DEPTH 溢出、PendingEdits begin/commit
- [ ] `notes_key` — midi_version 变化时 key 跟着变
- [ ] 增量 rebuild vs 全量 rebuild 结果一致

## 关键约束（不可违反）

- `Arc<MidiFile>` COW 模式不变
- `Snapshot::capture` 和 `f(doc)` 之间不能修改 `documents[idx]`
- 每次只改一个模块，cargo check 通过后提交
- 不一次性改 179 处调用点
