import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import 'ffi.dart' as rust;
import 'automation_panel.dart';
import 'canvas_menus.dart';
import 'home_view.dart';
import 'models.dart';
import 'project_dialog.dart';
import 'painters.dart';
import 'settings_view.dart';
import 'theme.dart';
import 'tools.dart';
import 'track_info_dialog.dart';
import 'top_bar.dart';

/// 视图模式（与 Rust 侧约定一致：0=AR 走带，1=PR 卷帘）。
const int kViewArrangement = 0;
const int kViewPianoroll = 1;

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  // 沉浸式全屏：隐藏顶部状态栏与底部小白条（滑动可临时唤出）。
  SystemChrome.setEnabledSystemUIMode(SystemUiMode.immersiveSticky);
  SystemChrome.setSystemUIOverlayStyle(
    const SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      systemNavigationBarColor: Colors.transparent,
    ),
  );
  runApp(const YinheApp());
}

class YinheApp extends StatelessWidget {
  const YinheApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: YinheTheme.data(),
      home: const StudioShell(),
    );
  }
}

/// 工作台壳：画布常驻，主页为覆盖浮层，上侧栏为走带控制。
class StudioShell extends StatefulWidget {
  const StudioShell({super.key});

  @override
  State<StudioShell> createState() => _StudioShellState();
}

class _StudioShellState extends State<StudioShell>
    with SingleTickerProviderStateMixin, WidgetsBindingObserver {
  static const MethodChannel _channel = MethodChannel('yinhe/surface');

  int? _textureId;
  Size? _reportedPixelSize;

  bool _showHome = true;
  int _viewMode = kViewArrangement;
  String? _fileName;
  String? _soundFontName;

  /// AR 中点击进入 PR 的轨道（自动化栏/聚焦用）。
  int? _activeTrack;

  bool _playing = false;

  /// 时间码显示模式：false=秒数（默认），true=小节.拍.tick。
  bool _timecodeBar = false;

  /// 时间码文本（每帧刷新，只重建文本本身）。
  final ValueNotifier<String> _timecodeText = ValueNotifier('0:00.000');

  /// 设置页浮层（从主页进入）。
  bool _showSettings = false;

  List<RecentItem> _recent = const [];

  /// 自动化栏数据（当前轨道，仅 PB/CC/RPN）。
  List<AutomationLaneData> _autoLanes = const [];

  /// 力度条数据：(tick, length, velocity01, track)。
  List<(double, double, double, int)> _velocityBars = const [];

  /// 收起自动化栏（最大化画布）。
  bool _autoCollapsed = false;

  /// 当前工具（PR 5 种；AR 2 种；索引对应 `toolDefs`）。
  int _toolPr = 0;
  int _toolAr = 0;

  /// 量化档位显示（桌面：每文档瞬态，默认自动）。
  String _quantizeLabel = '自动';

  /// “显示轨道”过滤（-1 = 全部轨道）。
  int _displayTrack = -1;

  /// 歌曲标题（meta.name；空 = 未设置）。
  String _songTitle = '';

  /// AR 折叠态自动化竖线。
  List<ArAutomationLane> _arAutomation = const [];

  /// 覆盖层（选框/预览）与修订号。
  final ValueNotifier<OverlayData> _overlay = ValueNotifier(OverlayData.empty);
  int _overlayRev = 0;

  /// 文档/选区世代号：变化时重新读取轨道/自动化/力度数据。
  int _docGeneration = 0;

  /// 工程状态位：bit0=可撤销，bit1=可重做，bit2=未保存。
  final ValueNotifier<int> _docState = ValueNotifier(0);
  late final ffi.Pointer<ffi.Int32> _statePtr;

  final ValueNotifier<double> _playhead = ValueNotifier<double>(-1);
  final ValueNotifier<List<double>> _snap = ValueNotifier<List<double>>(
    List<double>.filled(12, 0.0),
  );
  final ValueNotifier<List<TrackRow>> _tracks = ValueNotifier<List<TrackRow>>(
    const [],
  );
  final ValueNotifier<int> _dragFrom = ValueNotifier<int>(-1);
  final ValueNotifier<int> _dragTo = ValueNotifier<int>(-1);

  late final Ticker _ticker;
  late final ffi.Pointer<ffi.Float> _snapPtr;

  bool _dragging = false;

  /// PR 选框长按：圆环进度（500ms）；完成后进入移动态，未移动则弹菜单。
  Offset? _lpPos;
  DateTime? _lpStartTime;
  bool _lpMoved = false;
  final ValueNotifier<double> _lpProgress = ValueNotifier(0);
  final ValueNotifier<Offset?> _lpPosNotifier = ValueNotifier(null);
  final ValueNotifier<bool> _moveDashed = ValueNotifier(false);

  /// 触控板捏合的上一次累计缩放（用于换算增量）。
  double _panZoomScale = 1.0;

  /// 帧计数（时间码/工程状态降频）。
  int _frameCounter = 0;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _snapPtr = calloc<ffi.Float>(12);
    _statePtr = calloc<ffi.Int32>(3);
    _channel.setMethodCallHandler(_onNativeCall);
    _init();
    _ticker = createTicker((_) {
      // 帧驱动同步：先让 wgpu 渲染当前状态的一帧（同步等待完成），再用同一状态
      // 刷新浮层，保证纹理与 Flutter 浮层出自同一份场景数据、同一帧合成。
      if (!_showHome) {
        // 拖拽边缘自动滚动：先滚动视图，再渲染这一帧。
        rust.dragTick();
        rust.renderFrame();
      }
      _playhead.value = rust.playheadX();
      final snap = _readSnapshot();
      if (!_sameSnapshot(snap)) {
        _snap.value = snap;
      }
      _readOverlay();
      // 选框状态驱动（Rust 计时）：移动态画虚线；长按中更新圆环进度。
      final selState = rust.selectionState();
      if (_moveDashed.value != (selState == 2)) {
        _moveDashed.value = selState == 2;
      }
      if (selState == 2 || selState == 3) {
        // 已转移动态/长按完成：清掉圆环（移动态虚线由 moving 分支绘制）。
        if (_lpProgress.value != 0) {
          _lpProgress.value = 0;
        }
      } else if (_lpStartTime != null && selState == 1) {
        // 前 200ms 不显示圆环（防普通点击遮挡内容），总时长仍为 500ms。
        final ms = DateTime.now().difference(_lpStartTime!).inMilliseconds;
        _lpProgress.value = ms <= 200 ? 0 : ((ms - 200) / 300).clamp(0.0, 1.0);
      }
      // 工程状态/时间码降频（每 ~66ms）：减少音频锁与历史栈分配开销。
      _frameCounter++;
      if (_frameCounter % 4 == 0) {
        final generation = rust.documentGeneration();
        if (generation != _docGeneration) {
          _docGeneration = generation;
          _refreshDocumentMeta();
        }
        _readDocState();
        _updateTimecode();
      }
      // 保持连续帧：Rust 侧拖拽/边缘滚动/渲染都由帧驱动（手指静止时也要跑）。
      if (!_showHome) {
        SchedulerBinding.instance.scheduleFrame();
      }
    })..start();
    _recent = _readRecent();
    rust.setQuantize(1, 1, 1); // 默认自动（与桌面一致）
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _ticker.dispose();
    _timecodeText.dispose();
    _playhead.dispose();
    _snap.dispose();
    _tracks.dispose();
    _dragFrom.dispose();
    _dragTo.dispose();
    _lpProgress.dispose();
    _lpPosNotifier.dispose();
    _moveDashed.dispose();
    _overlay.dispose();
    _docState.dispose();
    calloc.free(_snapPtr);
    calloc.free(_statePtr);
    super.dispose();
  }

  /// 从桌面/后台返回：重新应用沉浸式全屏（系统可能已重置）。
  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      SystemChrome.setEnabledSystemUIMode(SystemUiMode.immersiveSticky);
    } else {
      // 切后台/失去焦点时触点 up 可能收不到：清空，避免残留成“幽灵手指”。
      rust.resetTouch();
    }
  }

  // ── 数据读取 ──

  List<double> _readSnapshot() {
    if (rust.viewSnapshot(_snapPtr) == 0) {
      return _snap.value;
    }
    return List<double>.generate(12, (i) => _snapPtr[i]);
  }

  /// 视图快照是否与当前一致（避免无变化时触发整层重绘）。
  bool _sameSnapshot(List<double> next) {
    final cur = _snap.value;
    if (identical(cur, next)) {
      return true;
    }
    for (var i = 0; i < next.length; i++) {
      if (i >= cur.length || cur[i] != next[i]) {
        return false;
      }
    }
    return true;
  }

  /// 覆盖层：仅在修订号变化时重取 JSON。
  void _readOverlay() {
    final rev = rust.overlayRevision();
    if (rev == _overlayRev) {
      return;
    }
    _overlayRev = rev;
    final ptr = rust.overlayJson();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final json = jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
      _overlay.value = OverlayData.fromJson(json);
    } catch (_) {
      _overlay.value = OverlayData.empty;
    } finally {
      rust.freeString(ptr);
    }
  }

  /// 工程状态（撤销/重做可用性、未保存标记）。
  void _readDocState() {
    if (rust.docState(_statePtr) != 1) {
      return;
    }
    final bits =
        (_statePtr[0] & 1) |
        ((_statePtr[1] & 1) << 1) |
        ((_statePtr[2] & 1) << 2);
    if (_docState.value != bits) {
      _docState.value = bits;
    }
  }

  /// 当前视图允许的工具。
  int get _tool => _viewMode == kViewPianoroll ? _toolPr : _toolAr;

  /// 把当前视图的工具同步给 Rust。
  void _pushTool() {
    final list = _viewMode == kViewPianoroll ? toolPrAllowed : toolArAllowed;
    final t = list.contains(_tool) ? _tool : list.first;
    rust.setTool(t);
  }

  void _selectTool(int tool) {
    setState(() {
      if (_viewMode == kViewPianoroll) {
        _toolPr = tool;
      } else {
        _toolAr = tool;
      }
    });
    _pushTool();
  }

  /// 量化档位同步给 Rust（工具吸附用）。
  void _pushQuantize(String label) {
    if (label == '关闭') {
      rust.setQuantize(0, 1, 1);
    } else if (label.startsWith('1/')) {
      final den = int.tryParse(label.substring(2)) ?? 4;
      rust.setQuantize(2, 1, den);
    } else {
      rust.setQuantize(1, 1, 1);
    }
  }

  /// 新建空工程并进入工作区。
  void _newProject() {
    rust.newProject();
    rust.setViewMode(kViewArrangement);
    rust.setDisplayTrack(-1);
    rust.setActiveTrack(-1);
    rust.setTool(toolArAllowed.first);
    setState(() {
      _fileName = 'Untitled';
      _showHome = false;
      _viewMode = kViewArrangement;
      _activeTrack = null;
      _displayTrack = -1;
      _toolAr = 0;
      _songTitle = '';
      _playing = false;
    });
    _updateTimecode();
    _ensureSurface();
  }

  /// 打开工程设置对话框（歌名/艺术家/PPQ/简介）。
  void _openProjectSettings() {
    final ptr = rust.projectMetaJson();
    Map<String, dynamic> meta = const {};
    if (ptr != ffi.nullptr) {
      try {
        meta = jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
      } catch (_) {
        meta = const {};
      } finally {
        rust.freeString(ptr);
      }
    }
    if (meta.isEmpty) {
      ScaffoldMessenger.of(context)
          .showSnackBar(const SnackBar(content: Text('请先打开 MIDI 或新建工程')));
      return;
    }
    rust.resetTouch();
    showDialog<void>(
      context: context,
      builder: (_) => ProjectDialog(
        initialName: meta['name'] as String? ?? '',
        initialArtist: meta['artist'] as String? ?? '',
        initialDescription: meta['description'] as String? ?? '',
        initialPpq: (meta['ppq'] as num?)?.toInt() ?? 480,
        onSave: (name, artist, desc, ppq) {
          final n = name.toNativeUtf8();
          final a = artist.toNativeUtf8();
          final d = desc.toNativeUtf8();
          try {
            if (rust.setProjectMeta(n, a, d, ppq) == 1) {
              _refreshDocumentMeta();
            }
          } finally {
            calloc.free(n);
            calloc.free(a);
            calloc.free(d);
          }
        },
      ),
    );
  }

  /// 导出 .ying 工程：Rust 写临时文件 → Kotlin 复制到用户选择的位置。
  Future<void> _saveProject({bool saveAs = false}) async {
    if (_tracks.value.isEmpty) {
      ScaffoldMessenger.of(context)
          .showSnackBar(const SnackBar(content: Text('请先打开 MIDI 或新建工程')));
      return;
    }
    final name = _songTitle.isNotEmpty ? _songTitle : (_fileName ?? 'Untitled');
    final tmp = await _channel.invokeMethod<String>('exportTempPath');
    if (tmp == null || !mounted) {
      return;
    }
    final p = tmp.toNativeUtf8();
    var ok = 0;
    try {
      ok = rust.saveYing(p);
    } finally {
      calloc.free(p);
    }
    if (ok != 1) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(const SnackBar(content: Text('工程导出失败')));
      }
      return;
    }
    await _channel.invokeMethod<void>('saveProject', {
      'name': name,
      'saveAs': saveAs,
    });
  }

  List<RecentItem> _readRecent() {
    final ptr = rust.recentJson();
    if (ptr == ffi.nullptr) {
      return const [];
    }
    try {
      final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
      return [
        for (final e in list) RecentItem.fromJson(e as Map<String, dynamic>),
      ];
    } catch (_) {
      return const [];
    } finally {
      rust.freeString(ptr);
    }
  }

  List<TrackRow> _readTracks() {
    final ptr = rust.tracksJson();
    if (ptr == ffi.nullptr) {
      return const [];
    }
    try {
      final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
      return [
        for (final e in list) TrackRow.fromJson(e as Map<String, dynamic>),
      ];
    } catch (_) {
      return const [];
    } finally {
      rust.freeString(ptr);
    }
  }

  /// 文档加载是后台线程：延迟刷新轨道/最近列表。
  void _refreshDocumentMeta() {
    if (!mounted) {
      return;
    }
    setState(() {
      _recent = _readRecent();
      _tracks.value = _readTracks();
    });
    _loadAutomation();
    _loadVelocity();
    _loadSongTitle();
    _loadArAutomation();
  }

  Future<void> _onNativeCall(MethodCall call) async {
    if (!mounted) {
      return;
    }
    final path = call.arguments as String?;
    if (path == null) {
      return;
    }
    switch (call.method) {
      case 'onFilePicked':
        setState(() {
          _fileName = path.split('/').last;
          _showHome = false;
          _viewMode = kViewArrangement;
          _activeTrack = null;
          _displayTrack = -1;
        });
        rust.setViewMode(kViewArrangement);
        rust.setDisplayTrack(-1);
        rust.setActiveTrack(-1);
        rust.setTool(toolArAllowed.first);
        _ensureSurface();
        _rememberProject(path.split('/').last);
      case 'onSoundFontPicked':
        setState(() => _soundFontName = path.split('/').last);
      case 'onProjectSaved':
        rust.markSaved();
        _loadSongTitle();
        _readDocState();
    }
  }

  Future<void> _init() async {
    final id = await _channel.invokeMethod<int>('getTextureId');
    if (!mounted) {
      return;
    }
    setState(() => _textureId = id);
    _loadSoundFontName();
    await _ensureSurface();
  }

  /// 确保 Rust 已拿到生产端 surface（部分引擎版本不触发 SurfaceProducer 回调）。
  Future<void> _ensureSurface() async {
    await _channel.invokeMethod<void>('ensureSurface');
  }

  /// 启动时同步已持久化的音色库名（Rust 侧已自动恢复音色库）。
  void _loadSoundFontName() {
    final ptr = rust.soundfontPath();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final path = ptr.toDartString();
      if (path.isEmpty || !mounted) {
        return;
      }
      setState(() => _soundFontName = path.split('/').last);
    } finally {
      rust.freeString(ptr);
    }
  }

  Future<void> _pickMidi() async {
    await _channel.invokeMethod<void>('openFilePicker');
  }

  Future<void> _pickSoundFont() async {
    await _channel.invokeMethod<void>('openSoundFontPicker');
  }

  void _openRecent(RecentItem item) {
    final p = item.path.toNativeUtf8();
    try {
      rust.openPath(p);
    } finally {
      calloc.free(p);
    }
    setState(() {
      _fileName = item.name;
      _showHome = false;
      _viewMode = kViewArrangement;
      _activeTrack = null;
      _displayTrack = -1;
    });
    rust.setViewMode(kViewArrangement);
    rust.setDisplayTrack(-1);
    rust.setActiveTrack(-1);
    rust.setTool(toolArAllowed.first);
    _ensureSurface();
    _rememberProject(item.name);
  }

  /// 按文件名恢复工程 SAF uri（最近列表打开 .ying 后可直接覆盖保存）。
  void _rememberProject(String name) {
    _channel.invokeMethod<void>('rememberProject', {'name': name});
  }

  void _openSettings() {
    setState(() => _showSettings = true);
  }

  /// 读取当前轨道的自动化 lane（仅 PB/CC/RPN）。
  void _loadAutomation() {
    final rows = _tracks.value;
    if (rows.isEmpty) {
      setState(() => _autoLanes = const []);
      return;
    }
    final track = (_activeTrack ?? 0).clamp(0, rows.length - 1);
    final ptr = rust.automationJson(track);
    var lanes = const <AutomationLaneData>[];
    if (ptr != ffi.nullptr) {
      try {
        final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
        lanes = [
          for (final e in list)
            AutomationLaneData.fromJson(e as Map<String, dynamic>),
        ];
      } catch (_) {
        lanes = const [];
      } finally {
        rust.freeString(ptr);
      }
    }
    if (!mounted) {
      return;
    }
    setState(() => _autoLanes = lanes);
  }

  /// 读取当前轨道的力度条数据（每音符一条）。
  void _loadVelocity() {
    final rows = _tracks.value;
    if (rows.isEmpty) {
      setState(() => _velocityBars = const []);
      return;
    }
    final ptr = rust.velocityJson();
    var bars = const <(double, double, double, int)>[];
    if (ptr != ffi.nullptr) {
      try {
        final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
        bars = [
          for (final e in list)
            (
              ((e as List<dynamic>)[0] as num).toDouble(),
              (e[1] as num).toDouble(),
              (e[2] as num).toDouble(),
              (e[3] as num).toInt(),
            ),
        ];
      } catch (_) {
        bars = const [];
      } finally {
        rust.freeString(ptr);
      }
    }
    if (!mounted) {
      return;
    }
    setState(() => _velocityBars = bars);
  }

  /// 读取 AR 折叠态自动化竖线（所有可见轨）。
  void _loadArAutomation() {
    final ptr = rust.arAutomationJson();
    var lanes = const <ArAutomationLane>[];
    if (ptr != ffi.nullptr) {
      try {
        final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
        lanes = [
          for (final e in list)
            ArAutomationLane.fromJson(e as Map<String, dynamic>),
        ];
      } catch (_) {
        lanes = const [];
      } finally {
        rust.freeString(ptr);
      }
    }
    if (!mounted) {
      return;
    }
    setState(() => _arAutomation = lanes);
  }

  /// 读取歌曲标题（meta.name）。
  void _loadSongTitle() {
    final ptr = rust.songTitle();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final title = ptr.toDartString();
      if (!mounted) {
        return;
      }
      setState(() => _songTitle = title);
    } finally {
      rust.freeString(ptr);
    }
  }

  void _setAutoCollapsed(bool collapsed) {
    setState(() => _autoCollapsed = collapsed);
  }

  // ── 走带 ──

  void _togglePlay() {
    final next = !_playing;
    setState(() => _playing = next);
    rust.transport(next ? 0 : 1);
    _updateTimecode();
  }

  void _stopPlayback() {
    rust.transport(2);
    _updateTimecode();
    setState(() => _playing = false);
  }

  /// 每帧刷新时间码文本（只触发文本重建，不动整棵布局）。
  void _updateTimecode() {
    if (!mounted) {
      return;
    }
    final text = _timecodeBar
        ? _fmtBarBeat(rust.playheadTick(), _ppq)
        : _fmtTime(rust.playbackSeconds());
    if (_timecodeText.value != text) {
      _timecodeText.value = text;
    }
    final playing = rust.isPlaying() == 1;
    if (playing != _playing) {
      setState(() => _playing = playing);
    }
  }

  /// 秒数（`m:ss.mmm`）。
  String _fmtTime(double seconds) {
    final total = seconds.floor();
    final m = total ~/ 60;
    final s = total % 60;
    final ms = ((seconds - total) * 1000).floor();
    return '$m:${s.toString().padLeft(2, '0')}.${ms.toString().padLeft(3, '0')}';
  }

  /// 小节.拍.tick（1.0.000 风格：bar 1 起、beat 0 起；4/4 定拍）。
  String _fmtBarBeat(double tick, double ppq) {
    if (ppq <= 0) {
      return '1.0.000';
    }
    final ticksPerBar = ppq * 4;
    final bar = (tick / ticksPerBar).floor() + 1;
    final beat = ((tick % ticksPerBar) / ppq).floor();
    final inBeat = (tick % ppq).floor();
    return '$bar.$beat.${inBeat.toString().padLeft(3, '0')}';
  }

  double get _ppq {
    final v = _snap.value[8];
    return v > 0 ? v : 480;
  }

  // ── 触摸 / 手势 ──

  void _sendTouch(int kind, Offset p, int pointer) {
    // AR 长按拖动音轨/自动化锚点时无需转发 move（避免误平移），但 up/cancel
    // 必须转发：否则 Rust 触点表永久残留这根手指，下一次单指会被判成双指。
    if (_dragging && kind != 2) {
      return;
    }
    // 只写入事件；纹理与浮层统一在下一帧同步刷新（帧驱动，输入侧已就地 clamp）。
    if (kind == 2) {
      // up 前读取状态与长按上下文（Rust 在 up 时会清理）。
      final state = rust.selectionState();
      LongPressContext? ctx;
      if (state == 3) {
        final ptr = rust.longPressContextJson();
        if (ptr != ffi.nullptr) {
          try {
            final decoded = jsonDecode(ptr.toDartString());
            if (decoded is Map<String, dynamic>) {
              ctx = LongPressContext.fromJson(decoded);
            }
          } catch (_) {
            ctx = null;
          } finally {
            rust.freeString(ptr);
          }
        }
      }
      rust.touch(kind, p.dx, p.dy, pointer);
      _finishLongPressWatch(state, ctx);
    } else {
      rust.touch(kind, p.dx, p.dy, pointer);
      if (kind == 0) {
        // 选框内/空白处按下由 Rust 进入 PENDING（500ms 后自行转移动态或粘贴态）。
        if (rust.selectionState() == 1) {
          _beginLongPressWatch(p);
        }
      } else if (kind == 1) {
        _updateLongPressWatch(p);
      }
    }
    // 触摸期间确保有下一帧（拖拽边缘自动滚动/预览依赖帧驱动）。
    SchedulerBinding.instance.scheduleFrame();
  }

  /// 按下：开始长按计时（圆环转满后进入移动态或粘贴态）。
  ///
  /// 判定由 Rust 的 PENDING 状态给出；500ms 计时也在 Rust 帧循环里推进。
  void _beginLongPressWatch(Offset p) {
    _lpPos = p;
    _lpPosNotifier.value = p;
    _lpStartTime = DateTime.now();
    _lpMoved = false;
    _lpProgress.value = 0;
    _moveDashed.value = false;
  }

  void _updateLongPressWatch(Offset p) {
    if (_lpStartTime == null) {
      return;
    }
    final state = rust.selectionState();
    if (state == 1) {
      // 长按未完成就移动：取消（避免误拖）。
      if ((p - _lpPos!).distance > 12) {
        _cancelLongPressWatch();
      }
    } else if (state == 2) {
      // 已进入移动态：超过手指抖动阈值才算真的移动（否则松手仍弹复制菜单）。
      if (_lpPos != null && (p - _lpPos!).distance > 12) {
        _lpMoved = true;
      }
    } else if (state == 0) {
      // Rust 已把空白长按转成普通框选：只收掉长按 UI，不取消框选。
      _clearLongPressUi();
    }
  }

  /// 清理长按 UI 状态（不改 Rust 拖拽状态）。
  void _clearLongPressUi() {
    _lpStartTime = null;
    _lpPos = null;
    _lpPosNotifier.value = null;
    _lpProgress.value = 0;
    _lpMoved = false;
  }

  void _cancelLongPressWatch() {
    if (_lpStartTime == null) {
      return;
    }
    _clearLongPressUi();
    _moveDashed.value = false;
    rust.cancelSelectionPending();
  }

  void _finishLongPressWatch(int state, LongPressContext? ctx) {
    if (_lpStartTime == null) {
      return;
    }
    final moved = _lpMoved;
    final pos = _lpPos;
    _clearLongPressUi();
    // 虚线由 Ticker 依 state 关闭（up 后 state=0）。
    if (state == 2 && !moved) {
      _showSelectionMenu();
    } else if (state == 3) {
      _showPasteMenu(pos, ctx);
    }
  }

  /// 长按完成但未移动：选区操作菜单。
  void _showSelectionMenu() {
    rust.resetTouch();
    showSelectionMenu(
      context,
      onCopy: () {
        if (rust.copySelection() == 1) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(
              content: Text('已复制'),
              duration: Duration(seconds: 1),
            ),
          );
        }
      },
    );
  }

  /// 空白处长按完成：粘贴/插入菜单（都在长按位置操作）。
  void _showPasteMenu(Offset? pos, LongPressContext? ctx) {
    if (pos == null) {
      return;
    }
    rust.resetTouch();
    showEmptyLongPressMenu(
      context,
      canPaste: rust.clipboardEmpty() == 0,
      onPaste: () {
        if (rust.pasteSelection(pos.dx) == 1) {
          _refreshDocumentMeta();
        }
      },
      onInsert: ctx == null ? null : () => _showInsertMenu(ctx),
    );
  }

  /// 长按"插入"：Master 轨给全局事件，普通轨给通道事件。
  void _showInsertMenu(LongPressContext ctx) {
    showInsertMenu(
      context,
      conductor: ctx.conductor,
      onPick: (kind) {
        if (rust.insertAutomation(kind, ctx.tick, ctx.track) == 1) {
          _refreshDocumentMeta();
        }
      },
    );
  }

  /// 鼠标滚轮 / 触控板滚动：Ctrl（或 Cmd）+ 滚轮缩放（键盘区缩放音高轴，
  /// 其余缩放时间轴），普通滚轮平移时间轴。
  void _handlePointerSignal(PointerSignalEvent e) {
    if (e is! PointerScrollEvent) {
      return;
    }
    final ctrl =
        HardwareKeyboard.instance.isControlPressed ||
        HardwareKeyboard.instance.isMetaPressed;
    rust.scrollAt(
      e.localPosition.dx,
      e.localPosition.dy,
      e.scrollDelta.dx,
      e.scrollDelta.dy,
      ctrl ? 1 : 0,
    );
  }

  /// 触控板捏合：按累计缩放的增量缩放（键盘区 → 音高轴）。
  void _handlePanZoomUpdate(PointerPanZoomUpdateEvent e) {
    final next = e.scale;
    final last = _panZoomScale;
    _panZoomScale = next;
    if (last <= 0 || next <= 0) {
      return;
    }
    final factor = next / last;
    if ((factor - 1.0).abs() > 0.002) {
      rust.zoomAt(e.localPosition.dx, e.localPosition.dy, factor);
    }
  }

  int? _rowAt(Offset p) {
    final s = _snap.value;
    final panel = s[5];
    final laneH = s[6];
    final scrollY = s[3];
    if (laneH <= 0 || p.dx >= panel) {
      return null;
    }
    final idx = ((p.dy + scrollY) / laneH).floor();
    final rows = _tracks.value;
    if (idx < 0 || idx >= rows.length) {
      return null;
    }
    return idx;
  }

  bool _inMsZone(Offset p) {
    final s = _snap.value;
    final panel = s[5];
    final laneH = s[6];
    if (laneH < 34) {
      return false;
    }
    final btn = (laneH * 0.44).clamp(18.0, 26.0).toDouble();
    final zoneLeft = panel - 6.0 - btn * 2 - 4.0 - 4.0;
    return p.dx >= zoneLeft;
  }

  /// AR：点击轨道面板（M/S 之外）进入 PR。
  void _handleCanvasTap(Offset p) {
    if (_viewMode != kViewArrangement) {
      return;
    }
    if (_inMsZone(p)) {
      return;
    }
    final idx = _rowAt(p);
    if (idx == null) {
      return;
    }
    _activeTrack = idx;
    rust.setActiveTrack(idx);
    _loadAutomation();
    _loadVelocity();
    _setView(kViewPianoroll);
  }

  /// AR：长按拖动轨道排序。
  void _handleLongPressStart(Offset p) {
    if (_viewMode != kViewArrangement) {
      return;
    }
    final idx = _rowAt(p);
    if (idx == null) {
      return;
    }
    setState(() {
      _dragFrom.value = idx;
      _dragTo.value = idx;
      _dragging = true;
    });
  }

  void _handleLongPressMove(Offset p) {
    // Master（Conductor）固定在 0 号位，拖动不改目标。
    if (_dragFrom.value <= 0) {
      return;
    }
    final idx = _rowAt(p);
    if (idx != null && idx != _dragTo.value) {
      setState(() => _dragTo.value = idx);
    }
  }

  void _handleLongPressEnd() {
    final from = _dragFrom.value;
    final to = _dragTo.value;
    setState(() {
      _dragFrom.value = -1;
      _dragTo.value = -1;
      _dragging = false;
    });
    if (from < 0) {
      return;
    }
    if (from == 0) {
      // Master 轨只弹信息菜单，不参与排序。
      _showTrackMenu(0);
      return;
    }
    if (to < 0 || from == to) {
      // 长按未拖动：弹出音轨信息菜单。
      _showTrackMenu(from);
      return;
    }
    rust.moveTrack(from, to);
  }

  /// AR 长按音轨（未拖动）：音轨信息菜单。
  void _showTrackMenu(int track) {
    rust.resetTouch();
    showTrackMenu(context, onEdit: () => _editTrackInfo(track));
  }

  /// 编辑音轨信息（名称/端口/通道/本通道乐器）。
  void _editTrackInfo(int track) {
    final rows = _tracks.value;
    if (track < 0 || track >= rows.length) {
      return;
    }
    rust.resetTouch();
    final row = rows[track];
    var instruments = const <(int, int)?>[];
    final instPtr = rust.channelInstrumentsJson();
    if (instPtr != ffi.nullptr) {
      try {
        final list = jsonDecode(instPtr.toDartString()) as List<dynamic>;
        instruments = [
          for (final e in list)
            e == null
                ? null
                : (
                    ((e as List<dynamic>)[0] as num).toInt(),
                    (e[1] as num).toInt(),
                  ),
        ];
      } catch (_) {
        instruments = const [];
      } finally {
        rust.freeString(instPtr);
      }
    }
    showDialog<void>(
      context: context,
      builder: (_) => TrackInfoDialog(
        initialName: row.name,
        initialPort: row.port,
        initialChannel: row.channel,
        isMaster: row.master,
        channelInstruments: instruments,
        onSave: (name, port, channel, bank, program) {
          final p = name.toNativeUtf8();
          var ok = 0;
          try {
            ok = rust.setTrackInfo(
              track,
              p,
              port,
              channel,
              bank ?? 0,
              program ?? 0,
              bank != null ? 1 : 0,
            );
          } finally {
            calloc.free(p);
          }
          if (ok == 1) {
            _refreshDocumentMeta();
          }
        },
      ),
    );
  }

  void _setView(int mode) {
    if (_viewMode == mode) {
      return;
    }
    rust.setViewMode(mode);
    setState(() => _viewMode = mode);
    _pushTool();
  }

  // ── 布局 ──

  @override
  Widget build(BuildContext context) {
    final id = _textureId;
    if (id == null) {
      return const Scaffold(
        backgroundColor: Color(0xFFF0F0F4),
        body: Center(child: CircularProgressIndicator()),
      );
    }
    return Scaffold(
      backgroundColor: YinheTheme.bg,
      body: SafeArea(
        child: Stack(
          children: [
            Column(
              children: [
                // 顶部栏占位（栏本体在外层 Stack 顶层，保证绘制优先级最高）。
                const SizedBox(height: 42),
                Expanded(
                  child: LayoutBuilder(
                    builder: (context, constraints) {
                      final size = constraints.biggest;
                      // 内容画布 = 画布区减去顶部标尺条（Rust 坐标系从标尺下方开始）。
                      final contentH = (size.height - RulerPainter.height)
                          .clamp(1.0, double.infinity);
                      final dpr = MediaQuery.devicePixelRatioOf(context);
                      final pixelW = (size.width * dpr).round();
                      final pixelH = (contentH * dpr).round();
                      if (_reportedPixelSize?.width != pixelW ||
                          _reportedPixelSize?.height != pixelH) {
                        _reportedPixelSize = Size(
                          pixelW.toDouble(),
                          pixelH.toDouble(),
                        );
                        _channel.invokeMethod<void>('setSize', {
                          'w': pixelW,
                          'h': pixelH,
                          'lw': size.width,
                          'lh': contentH,
                        });
                      }
                      return Stack(
                        children: [
                          Positioned(
                            left: 0,
                            right: 0,
                            top: RulerPainter.height,
                            bottom: 0,
                            child: GestureDetector(
                              behavior: HitTestBehavior.translucent,
                              onTapUp: (d) => _handleCanvasTap(d.localPosition),
                              onLongPressStart: (d) =>
                                  _handleLongPressStart(d.localPosition),
                              onLongPressMoveUpdate: (d) =>
                                  _handleLongPressMove(d.localPosition),
                              onLongPressEnd: (_) => _handleLongPressEnd(),
                              child: Listener(
                                behavior: HitTestBehavior.opaque,
                                onPointerDown: (e) =>
                                    _sendTouch(0, e.localPosition, e.pointer),
                                onPointerMove: (e) =>
                                    _sendTouch(1, e.localPosition, e.pointer),
                                onPointerUp: (e) =>
                                    _sendTouch(2, e.localPosition, e.pointer),
                                onPointerCancel: (e) =>
                                    _sendTouch(2, e.localPosition, e.pointer),
                                onPointerSignal: _handlePointerSignal,
                                onPointerPanZoomStart: (_) =>
                                    _panZoomScale = 1.0,
                                onPointerPanZoomUpdate: _handlePanZoomUpdate,
                                onPointerPanZoomEnd: (_) => _panZoomScale = 1.0,
                                child: Texture(textureId: id),
                              ),
                            ),
                          ),
                          // 层级：音符（Texture）→ 演奏指示线 → 音轨面板 → 时间标尺 → 自动化栏。
                          if (!_showHome)
                            Positioned.fill(
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: PlayheadPainter(_playhead),
                                ),
                              ),
                            ),
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: RulerPainter.height,
                              bottom: 0,
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: LabelsPainter(
                                    snap: _snap,
                                    tracks: _tracks,
                                    dragFrom: _dragFrom,
                                    dragTo: _dragTo,
                                    arAutomation: _arAutomation,
                                  ),
                                ),
                              ),
                            ),
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: RulerPainter.height,
                              bottom: 0,
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: OverlayPainter(
                                    _overlay,
                                    _moveDashed,
                                  ),
                                ),
                              ),
                            ),
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: RulerPainter.height,
                              bottom: 0,
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: SelectionLongPressPainter(
                                    _lpProgress,
                                    _lpPosNotifier,
                                    _moveDashed,
                                    _overlay,
                                  ),
                                ),
                              ),
                            ),
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: 0,
                              height: RulerPainter.height,
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: RulerPainter(_snap),
                                ),
                              ),
                            ),
                          if (!_showHome && _viewMode == kViewPianoroll)
                            if (!_autoCollapsed)
                              AutomationPanel(
                                snap: _snap,
                                lanes: _autoLanes,
                                velocity: _velocityBars,
                                trackColors: [
                                  for (final t in _tracks.value) t.color,
                                ],
                                activeTrack: _activeTrack ?? 0,
                                onCollapsed: _setAutoCollapsed,
                                onReload: _loadAutomation,
                              )
                            else
                              AutoExpandButton(
                                onTap: () => _setAutoCollapsed(false),
                              ),
                        ],
                      );
                    },
                  ),
                ),
              ],
            ),
            if (!_showHome)
              Positioned(
                left: 0,
                right: 0,
                top: 0,
                height: 42,
                child: _buildTopBar(),
              ),
            if (_showHome)
              HomeView(
                recent: _recent,
                onPickMidi: _pickMidi,
                onNewProject: _newProject,
                onProjectSettings: _openProjectSettings,
                onSaveProject: () => _saveProject(),
                onSaveProjectAs: () => _saveProject(saveAs: true),
                onOpenRecent: _openRecent,
                onOpenSettings: _openSettings,
                onBack: () => setState(() => _showHome = false),
              ),
            if (_showSettings)
              SettingsView(
                soundFontName: _soundFontName,
                onPickSoundFont: _pickSoundFont,
                onBack: () => setState(() => _showSettings = false),
              ),
          ],
        ),
      ),
    );
  }

  /// 上侧栏（实现见 top_bar.dart）。
  Widget _buildTopBar() {
    return TopBar(
      isPr: _viewMode == kViewPianoroll,
      playing: _playing,
      timecodeText: _timecodeText,
      quantizeLabel: _quantizeLabel,
      ppq: _ppq,
      tool: _tool,
      tracks: _tracks.value,
      displayTrack: _displayTrack,
      songTitle: _songTitle,
      fileName: _fileName,
      docState: _docState,
      onHome: () => setState(() => _showHome = true),
      onBack: () => _setView(kViewArrangement),
      onTogglePlay: _togglePlay,
      onStop: _stopPlayback,
      onToggleTimecode: () {
        setState(() => _timecodeBar = !_timecodeBar);
        _updateTimecode();
      },
      onQuantize: (v) {
        setState(() => _quantizeLabel = v);
        _pushQuantize(v);
      },
      onTool: _selectTool,
      onUndo: _undo,
      onRedo: _redo,
      onSaveAs: () => _saveProject(saveAs: true),
      onSelectDisplayTrack: _selectDisplayTrack,
    );
  }

  /// 选择“显示轨道”（-1 = 全部）；同步自动化/力度条跟随。
  void _selectDisplayTrack(int track) {
    setState(() {
      _displayTrack = track;
      if (track >= 0) {
        _activeTrack = track;
      }
    });
    rust.setDisplayTrack(track);
    rust.setActiveTrack(track);
    _loadAutomation();
    _loadVelocity();
  }

  /// 撤销一步（含快照/音频刷新）。
  void _undo() {
    if (rust.undo() == 1) {
      _refreshDocumentMeta();
    }
  }

  /// 重做一步（含快照/音频刷新）。
  void _redo() {
    if (rust.redo() == 1) {
      _refreshDocumentMeta();
    }
  }
}
