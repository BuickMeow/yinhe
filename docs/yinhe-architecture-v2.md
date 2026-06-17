# Yinhe Architecture v2 — 极简版

> 从「基于 MidiFile 的中转架构」迁移到「YinModel 自研内核」。
>
> **原则**：删了重写，不并行 v1/v2，不预留未来扩展点，没有症状的优化就是废代码。

---

## 1. 目标

```
[.mid]  --parse-->  YinModel (内存)  --bincode+zstd-->  [.yin]
[.yin]  --unzstd--> YinModel
                      |
                      +--> GPU (PianoRoll, NoteSource trait)
                      +--> XSynth (实时调度，audio thread 读 ArcSwap 快照)
```

**YinModel 是唯一内存模型。MidiFile 删除。MidiControlEvent 删除。**

---

## 2. 数据模型

### YinModel

```rust
pub struct YinModel {
    pub conductor:   Arc<ConductorData>,
    pub tracks:      Vec<Arc<TrackData>>,        // C1：每 track 独立 Arc
    pub tempo_map:   Arc<TempoMap>,              // 派生
    pub meta:        ProjectMeta,
    // 派生索引（rebuild 时全量构建）
    pub key_notes_cache:  Vec<Vec<Note>>,        // 128 vec, NoteSource 兼容
    pub note_count:       u64,
    pub tick_length:      u64,
}
```

### TrackData

```rust
pub struct TrackData {
    pub uuid:           String,
    pub name:           String,
    pub color:          [f32; 3],
    pub port:           u8,         // 0..16  (A..P)
    pub channel:        u8,         // 0..16  (1..16 显示)
    pub channel_prefix: Option<u8>,
    pub muted:          bool,
    pub soloed:         bool,
    pub notes:          Vec<NoteEvent>,
    pub cc:             BTreeMap<u8, Vec<CcEvent>>,
    pub pitch_bend:     Vec<PitchBendEvent>,
    pub program_change: Vec<PcEvent>,
    pub rpn:            BTreeMap<u16, Vec<RpnEvent>>,  // key = (msb<<8)|lsb
}
```

### Events

```rust
pub struct NoteEvent {
    pub start_tick: u32,
    pub end_tick:   u32,           // 内存用 end_tick，播放友好
    pub key:        u8,
    pub velocity:   u8,
    pub dup_index:  u8,            // 同 (key, start_tick) 重叠序号
}

pub struct CcEvent        { pub tick: u32, pub value: u8 }
pub struct PitchBendEvent { pub tick: u32, pub value: i16 }   // -8192..8191
pub struct PcEvent        { pub tick: u32, pub program: u8 }
pub struct RpnEvent       { pub tick: u32, pub value: u16 }
```

**NoteEvent 不带 channel/track**，由所属 TrackData 隐含。

### ConductorData

```rust
pub struct ConductorData {
    pub tempo:    Vec<TempoEvent>,
    pub time_sig: Vec<TimeSigEvent>,
}

pub struct TempoEvent   { pub tick: u32, pub bpm: f64 }
pub struct TimeSigEvent { pub tick: u32, pub numerator: u8, pub denominator: u8 }
```

不预留 markers / key_signature。需要时再加。

### TempoMap

从旧 MidiFile 迁移的时间映射，独立类型。

```rust
pub struct TempoMap {
    pub ticks_per_beat: u32,
    pub tempo_segments: Vec<TempoSegment>,
    pub time_sig_events: Vec<TimeSigEvent>,
    pub time_sig_default: (u8, u8),
    pub tick_length: u64,
}

impl TempoMap {
    pub fn tick_to_seconds(&self, tick: u64) -> f64;
    pub fn tick_at_time(&self, time: f64) -> f64;
    pub fn bpm_at_time(&self, time: f64) -> f32;
    pub fn bar_divide(&self) -> f64;
    pub fn bar_at_tick(&self, tick: u64) -> u64;
    pub fn total_bars(&self) -> u64;
    pub fn time_sig_at_tick(&self, tick: u32) -> (u8, u8);
}
```

---

## 3. .yin 文件格式

```
+-------------------------+
|  Header                 |  magic + version + 3 段长度
+-------------------------+
|  project.json           |  人类可读元数据
+-------------------------+
|  mapping.json           |  轨道树 + soundfont + view
+-------------------------+
|  data.bin               |  bincode(model_data) -> zstd
+-------------------------+
```

### Container layout

```
magic:        b"YINH"            (4 bytes)
version:      u16 LE             (2 bytes)
project_len:  u32 LE             (4 bytes)
project_json: [u8; project_len]  (utf-8 JSON)
mapping_len:  u32 LE
mapping_json: [u8; mapping_len]  (utf-8 JSON)
data_len:     u32 LE
data:         [u8; data_len]     (zstd of bincode(ModelData))
```

没有索引区。要读 mapping 必须先读 project_json，要读 tracks 必须解 zstd 整段。简单。

### project.json

```json
{
  "version": 2,
  "name": "...",
  "artist": "...",
  "description": "...",
  "ppq": 480,
  "compression_level": 3
}
```

### mapping.json

```json
{
  "version": 2,
  "ports": [
    {
      "port": 0,
      "channels": [
        {
          "channel": 0,
          "tracks": [
            {
              "uuid": "...",
              "name": "...",
              "color": [0.5, 0.5, 0.5],
              "channel_prefix": null,
              "muted": false,
              "soloed": false
            }
          ]
        }
      ]
    }
  ],
  "soundfonts": {
    "0": ["path/to/sf1.sf2"]
  },
  "view": {
    "zoom_x": 1.0,
    "zoom_y": 1.0,
    "scroll_tick": 0,
    "scroll_key": 60,
    "active_track_uuid": null
  }
}
```

为什么 mapping.json 单独存在：UI 加载时要快速展示轨道列表（颜色/名字/SF 配置），不需要解整个 zstd 数据流。

### data.bin

```rust
#[derive(Serialize, Deserialize)]
struct ModelData {
    conductor: ConductorData,
    tracks: Vec<TrackPayload>,        // 顺序与 mapping.json 一致
}

#[derive(Serialize, Deserialize)]
struct TrackPayload {
    uuid: String,                     // 校验用
    notes: Vec<NoteEvent>,
    cc: BTreeMap<u8, Vec<CcEvent>>,
    pitch_bend: Vec<PitchBendEvent>,
    program_change: Vec<PcEvent>,
    rpn: BTreeMap<u16, Vec<RpnEvent>>,
}
```

`bincode::serialize(&ModelData)` -> `zstd::encode(level)` -> 写入 data 段。完。

**没有列式存储。没有每字段独立流。** 之前 delta+gate+zstd 已经压了 98.5%，列式是 marginal 收益 + 巨大复杂度。

---

## 4. 并发模型

```rust
struct Document {
    model: arc_swap::ArcSwap<YinModel>,
    history: UndoStack,
    edit:    EditState,
}
```

编辑路径（C1 + ArcSwap）：

```rust
let current = doc.model.load_full();
let mut new_model = (*current).clone();              // 浅拷贝
let track = Arc::make_mut(&mut new_model.tracks[i]); // 仅 clone 该 track
track.notes.insert(...);
new_model.rebuild();                                  // 全量 rebuild ~10ms
doc.model.store(Arc::new(new_model));
```

Audio thread 每 buffer 开头 `model.load_full()`，整个 buffer 用这个快照。延迟 = 1 buffer ≈ 10–20 ms。

**没有 GC thread**：audio thread drop 旧 Arc 真卡了再加，没症状不加。

**没有 RebuildScope**：`rebuild()` 全量重建。某操作真卡了再针对它做增量。

---

## 5. 音频引擎

```rust
fn process(&mut self, buf: &mut [f32]) {
    let snap = self.model.load_full();

    // 处理已 schedule 的 note-off
    while let Some(top) = self.note_off_heap.peek() {
        if top.0.0 > self.cursor_end_tick { break; }
        let (end_tick, ch, key) = self.note_off_heap.pop().unwrap().0;
        send_note_off(ch, key, ...);
    }

    // 扫每 track 的窗口
    for track in &snap.tracks {
        if track.muted { continue; }
        let start = track.notes.partition_point(|n| n.start_tick < self.cursor_start);
        for n in &track.notes[start..] {
            if n.start_tick >= self.cursor_end { break; }
            send_note_on(track.channel, n.key, n.velocity, ...);
            self.note_off_heap.push(Reverse((n.end_tick, track.channel, n.key)));
        }
        // cc / pb / pc / rpn 同理
    }
}
```

`note_off_heap: BinaryHeap<Reverse<(u32, u8, u8)>>` 在 audio engine 内部维护。

---

## 6. 实施步骤（先搭隔壁房子，再拆旧的）

**核心策略**：旧 crate 照常工作不动，新建 3 个独立 crate（yinhe-core / yinhe-mid2 / yinhe-yin），每个写完独立编译 + 单测通过。三个新 crate 全部就绪后，**一次性切消费者并删旧的**。编译中断窗口 < 30 分钟。

```
旧 crate 照常工作 ──────────────────────┐
  yinhe-types / yinhe-midi /            │
  yinhe-project / yinhe-model           │
                                        │
新建（独立并行，不动旧的）：             │
  yinhe-core   (YinModel + TempoMap)    │
  yinhe-mid2   (.mid ↔ YinModel)        │
  yinhe-yin    (.yin ↔ YinModel)        │
     ↓ 全部就绪后                       │
  切换日：消费者切到 yinhe-core          ←─┘
  删 yinhe-model + MidiFile + ...
  yinhe-types 收缩回纯类型层
```

### Review 1: yinhe-core crate

新建 `crates/yinhe-core/`，独立的内核类型层。

- `model.rs`：YinModel / TrackData / ConductorData / NoteEvent (`start_tick + end_tick + key + vel + dup_index`) / CcEvent / PitchBendEvent / PcEvent / RpnEvent / ProjectMeta
- `tempo_map.rs`：TempoMap + TempoSegment + 时间映射方法（从 yinhe-midi 复制过来，原文件不动）
- `rebuild.rs`：`YinModel::rebuild()` 全量重建 key_notes_cache / note_count / tick_length
- `source.rs`：`impl NoteSource for YinModel`（用 yinhe-types 的 trait）
- 单测：构造小 YinModel，rebuild，校验派生数据

依赖：`yinhe-types`（用 Note / NoteSource / NoteScanIndex / TickBuckets / TimeSigEvent）

**验收**：cargo test -p yinhe-core 通过；旧代码完全不变，旧测试仍 31 通过。

### Review 2: yinhe-mid2 crate

新建 `crates/yinhe-mid2/`，从 .mid bytes 直接产出 yinhe_core::YinModel。

- `parser.rs`：parse_bytes(bytes) -> YinModel；NoteEvent 直接用 `start_tick + end_tick`，不经过 tick+duration 中转
- `writer.rs`：YinModel -> Vec<u8>（标准 SMF 格式）
- 内部可以复用 midly crate 解析底层，但聚合成 YinModel 时直接输出新结构
- 单测：parse + write roundtrip；与旧 yinhe-midi 对同一个 .mid 比对（音符数、tempo、CC 数等等价）

依赖：`yinhe-core`、`yinhe-types`、`midly`

**验收**：cargo test -p yinhe-mid2 通过；用真实小 .mid 验证。

### Review 3: yinhe-yin crate

新建 `crates/yinhe-yin/`，YinModel ↔ .yin 文件。

- `container.rs`：Header (magic + version + 3 段长度) 编/解码
- `mapping.rs`：mapping.json schema + serde
- `project_meta.rs`：project.json schema + serde
- `io.rs`：load_yin / save_yin（bincode + zstd 单流）
- 单测：roundtrip — 构造 YinModel → save_yin → load_yin → 比对相等

依赖：`yinhe-core`、`serde`、`serde_json`、`bincode`、`zstd`

**验收**：cargo test -p yinhe-yin 通过；100 万音符的 YinModel save 体积 < 旧 yin 同等档位（可抽样验证）。

### 切换日（一次性，编译中断窗口 < 30 分钟）

三个新 crate 全部 review 通过后，进行一次性切换：

1. `yinhe-editor-core`：Document 持 `ArcSwap<yinhe_core::YinModel>`，删 `Arc<MidiFile>` 字段
2. `yinhe-audio`：engine 改读 ArcSwap，note_off_heap 实现
3. `yinhe-egui`：file_loader / event_browser / channels_panel 切换
4. `yinhe-pianoroll / yinhe-arrangement`：NoteSource 路径不动（YinModel 实现了 NoteSource）
5. `yinhe-editor-core/playback.rs` 改读 TempoMap
6. 全工作区把 `yinhe_midi::MidiFile` 替换为 `yinhe_core::YinModel`，`yinhe_model::*` 替换为 `yinhe_core::*`
7. 删 `crates/yinhe-model/`、删 yinhe-midi 老 parser、删 yinhe-project conversion/、删 yinhe-types::MidiControlEvent
8. 工作区 Cargo.toml 改名：可选 `yinhe-mid2` → `yinhe-midi`、`yinhe-yin` → `yinhe-project`（覆盖旧 crate）

**验收**：
- `rg -F MidiFile` 全工作区返回空
- `rg -F MidiControlEvent` 全工作区返回空
- `cargo build --workspace --all-targets` 通过
- `cargo test --workspace` 通过
- 手测：加载 .mid，保存 .yin，重新加载 .yin，导出 .mid，全链路正常

---

## 7. 已被砍掉的过度设计（避免下次再来）

以下设计在迭代 Q&A 中被提出，最终因「过度工程 / 没症状先加」而否决：

| 砍掉的设计 | 砍的理由 |
|-----------|---------|
| 同 crate 内 v1/v2 并行 7 阶段（每阶段两头插脚） | 中间态屎山翻倍。改用「先搭隔壁房子（独立新 crate），再一刀切删旧的」。 |
| GC thread + SPSC 队列回收旧 Arc | audio thread drop 是否真卡未测过。没症状不加。 |
| RebuildScope 10 变体（TrackNotes / TrackCc / TrackPitchBend / TrackPc / TrackRpn / TrackAdd / TrackRemove / Multiple / Conductor / Full） | 全量 rebuild ~10ms 可接受。哪个操作真卡再针对那个做增量。 |
| 列式存储（每 track 5 个字段独立流 + 每 CC/RPN 号独立流）| delta+gate+zstd 已压 98.5%，列式 marginal 收益 + 复杂度爆炸。 |
| 索引区不压缩为 partial load 预留 | 「未来永远不来」。整文件单 zstd 流。 |
| 文件夹结构 A01-P16/{uuid}/notes/delta.bin 树 | 与列式存储一起砍。bincode 单文件。 |
| 自定义 binary container 带 IndexEntry 表 | 简化为 magic + 3 段长度前缀。 |
| EditPreview 预留接口 | 没用到的接口就是死代码。要做时再加。 |
| ConductorData 预留 markers / key_signature / end_of_song | 不预留。需要时再加字段。 |
| TickBuckets v2 / NoteScanIndex v2 等多套派生索引 | 用现有的，不改。 |
| automation_lanes_cache 派生 | YinModel rebuild 时构建一次足够。 |

---

## 8. 数据模型规范要点（精简）

### 同 (key, start_tick) 重叠音符

`dup_index: u8` 解决。99% 为 0，多个重叠按插入顺序递增。selection 用 `(track_idx, key, start_tick, dup_index)` 复合键。无显式 UUID，零额外内存。

### 端口/通道编号

- port: 0..15，文件夹名 A..P
- channel: 0..15 内部，1..16 显示（文件夹名 01..16）

### tick 精度

u32。落盘时 bincode 不做 delta 优化（zstd 自己学得到模式）。

### length vs end_tick

内存 end_tick（播放友好，无加法）。落盘也是 end_tick（bincode 序列化，zstd 学得到 start/end 的相关性）。如果实测压缩率差太多，再改成 delta+length（但目前不预设这个优化）。

