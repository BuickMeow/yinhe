import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import 'ffi.dart' as rust;
import 'home_view.dart';
import 'models.dart';
import 'painters.dart';
import 'settings_view.dart';
import 'theme.dart';

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

  /// combo 索引：0 = Velocity（默认），1..=N = 自动化 lane。
  int _autoIndex = 0;

  /// 力度条数据：(tick, length, velocity01, track)。
  List<(double, double, double, int)> _velocityBars = const [];

  /// 收起自动化栏（最大化画布）。
  bool _autoCollapsed = false;

  /// 拖动中的锚点预览（tick, value）。
  final ValueNotifier<(double, double)?> _autoPreview = ValueNotifier(null);
  bool _draggingAnchor = false;

  /// 当前工具（PR 5 种；AR 2 种；索引对应 `_toolDefs`）。
  int _toolPr = 0;
  int _toolAr = 0;

  /// 量化档位显示（桌面：每文档瞬态，默认自动）。
  String _quantizeLabel = '自动';

  /// “显示轨道”过滤（-1 = 全部轨道）。
  int _displayTrack = -1;

  /// 歌曲标题（meta.name；空 = 未设置）。
  String _songTitle = '';

  /// 覆盖层（选框/预览）与修订号。
  final ValueNotifier<OverlayData> _overlay = ValueNotifier(OverlayData.empty);
  int _overlayRev = 0;

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
      // 工程状态/时间码降频（每 ~66ms）：减少音频锁与历史栈分配开销。
      _frameCounter++;
      if (_frameCounter % 4 == 0) {
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
    _autoPreview.dispose();
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
  static const List<int> _toolPrAllowed = [0, 1, 2, 3, 4];
  static const List<int> _toolArAllowed = [0, 1];

  int get _tool => _viewMode == kViewPianoroll ? _toolPr : _toolAr;

  /// 把当前视图的工具同步给 Rust。
  void _pushTool() {
    final list = _viewMode == kViewPianoroll ? _toolPrAllowed : _toolArAllowed;
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
    rust.setTool(_toolArAllowed.first);
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
    Future.delayed(const Duration(milliseconds: 400), _refreshDocumentMeta);
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
    showDialog<void>(
      context: context,
      builder: (_) => _ProjectDialog(
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
  }

  Future<void> _onNativeCall(MethodCall call) async {
    final path = call.arguments as String?;
    if (!mounted || path == null) {
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
        rust.setTool(_toolArAllowed.first);
        _ensureSurface();
        _rememberProject(path.split('/').last);
        Future.delayed(const Duration(milliseconds: 800), _refreshDocumentMeta);
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
    rust.setTool(_toolArAllowed.first);
    _ensureSurface();
    _rememberProject(item.name);
    Future.delayed(const Duration(milliseconds: 800), _refreshDocumentMeta);
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
      setState(() {
        _autoLanes = const [];
        _autoIndex = 0;
      });
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
    setState(() {
      _autoLanes = lanes;
      _autoIndex = 0;
    });
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

  /// 锚点拖动：按屏幕 x 找最近锚点（离散 lane 的 tick 不变，仅改 value）。
  void _handleAnchorPanStart(DragStartDetails d) {
    if (_autoIndex < 1 || _autoIndex > _autoLanes.length) {
      return;
    }
    final s = _snap.value;
    final ppu = s[1];
    if (ppu <= 0) {
      return;
    }
    final scrollX = s[2];
    final kbW = s[5];
    final localX = d.localPosition.dx;
    double? nearest;
    var bestDist = 10.0;
    for (final p in _autoLanes[_autoIndex - 1].points) {
      final x = p.dx * ppu - scrollX + kbW - AutomationPainter.comboWidth;
      final dist = (x - localX).abs();
      if (dist < bestDist) {
        bestDist = dist;
        nearest = p.dx;
      }
    }
    final tick = nearest;
    if (tick == null) {
      return;
    }
    final value = 1.0 - ((d.localPosition.dy - 1) / 80.0);
    setState(() {
      _draggingAnchor = true;
      _autoPreview.value = (tick, value.clamp(0.0, 1.0));
    });
  }

  void _handleAnchorPanUpdate(DragUpdateDetails d) {
    final pv = _autoPreview.value;
    if (!_draggingAnchor || pv == null) {
      return;
    }
    final value = 1.0 - ((d.localPosition.dy - 1) / 80.0);
    setState(() => _autoPreview.value = (pv.$1, value.clamp(0.0, 1.0)));
  }

  void _handleAnchorPanEnd() {
    final pv = _autoPreview.value;
    final wasDragging = _draggingAnchor;
    setState(() {
      _draggingAnchor = false;
      _autoPreview.value = null;
    });
    if (!wasDragging || pv == null || _autoIndex < 1) {
      return;
    }
    final rows = _tracks.value;
    final track = (_activeTrack ?? 0).clamp(0, rows.length - 1);
    final ok = rust.setAutomationValue(track, _autoIndex, pv.$1.round(), pv.$2);
    if (ok == 1) {
      Future.delayed(const Duration(milliseconds: 250), _loadAutomation);
    }
  }

  Color _activeTrackColor() {
    final rows = _tracks.value;
    if (rows.isEmpty) {
      return YinheTheme.accent;
    }
    final i = (_activeTrack ?? 0).clamp(0, rows.length - 1);
    return rows[i].color;
  }

  /// PR 底部自动化栏（高 80 + 1px 分隔线；combo 列 60 宽，含收起按钮）。
  Widget _buildAutomationPanel() {
    final names = ['Velocity', for (final l in _autoLanes) l.name];
    final index = _autoIndex.clamp(0, names.length - 1).toInt();
    final lane = index >= 1 && index <= _autoLanes.length
        ? _autoLanes[index - 1]
        : null;
    final color = _activeTrackColor();
    return Positioned(
      left: 0,
      right: 0,
      bottom: 0,
      child: Container(
        height: 81,
        decoration: BoxDecoration(
          color: YinheTheme.bg,
          border: Border(top: BorderSide(color: YinheTheme.lineFg)),
        ),
        child: Row(
          children: [
            SizedBox(
              width: AutomationPainter.comboWidth,
              child: Column(
                children: [
                  // 收起（最大化画布；左下角有升起按钮）。
                  InkWell(
                    onTap: () => _setAutoCollapsed(true),
                    child: SizedBox(
                      height: 22,
                      width: double.infinity,
                      child: Icon(
                        Icons.keyboard_arrow_down,
                        size: 16,
                        color: YinheTheme.textMuted,
                      ),
                    ),
                  ),
                  Expanded(
                    child: PopupMenuButton<int>(
                      tooltip: '自动化类型',
                      padding: EdgeInsets.zero,
                      itemBuilder: (_) => [
                        for (var i = 0; i < names.length; i++)
                          PopupMenuItem(value: i, child: Text(names[i])),
                      ],
                      onSelected: (i) => setState(() => _autoIndex = i),
                      child: Center(
                        child: Text(
                          names[index],
                          textAlign: TextAlign.center,
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: YinheTheme.textLabel,
                            fontSize: 10,
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
            Expanded(
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onPanStart: _handleAnchorPanStart,
                onPanUpdate: _handleAnchorPanUpdate,
                onPanEnd: (_) => _handleAnchorPanEnd(),
                child: SizedBox.expand(
                  child: CustomPaint(
                    painter: AutomationPainter(
                      lane: lane,
                      velocity: _velocityBars,
                      trackColors: [for (final t in _tracks.value) t.color],
                      color: color,
                      snap: _snap,
                      preview: _autoPreview,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// 收起状态的升起按钮（屏幕左下角）。
  Widget _buildAutoExpandButton() {
    return Positioned(
      left: 8,
      bottom: 8,
      child: Material(
        color: YinheTheme.controlBg,
        borderRadius: BorderRadius.circular(6),
        elevation: 1,
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          onTap: () => _setAutoCollapsed(false),
          child: Padding(
            padding: const EdgeInsets.all(5),
            child: Icon(
              Icons.keyboard_arrow_up,
              size: 18,
              color: YinheTheme.textSecondary,
            ),
          ),
        ),
      ),
    );
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
    if (_dragging || _draggingAnchor) {
      return;
    }
    // 只写入事件；纹理与浮层统一在下一帧同步刷新（帧驱动，输入侧已就地 clamp）。
    rust.touch(kind, p.dx, p.dy, pointer);
    // 触摸期间确保有下一帧（拖拽边缘自动滚动/预览依赖帧驱动）。
    SchedulerBinding.instance.scheduleFrame();
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
    if (_dragFrom.value < 0) {
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
    if (from < 0 || to < 0 || from == to) {
      return;
    }
    final ok = rust.moveTrack(from, to);
    if (ok == 1) {
      Future.delayed(const Duration(milliseconds: 250), _refreshDocumentMeta);
    }
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
                // 顶部栏真实占位（主页时留空，画布尺寸恒定）。
                SizedBox(
                  height: 42,
                  child: _showHome ? const SizedBox.shrink() : _buildTopBar(),
                ),
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
                                    _snap,
                                    _tracks,
                                    _dragFrom,
                                    _dragTo,
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
                                  painter: OverlayPainter(_overlay),
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
                              _buildAutomationPanel()
                            else
                              _buildAutoExpandButton(),
                        ],
                      );
                    },
                  ),
                ),
              ],
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

  /// 上侧栏：左 = 视图入口（AR 主页 / PR 返回走带）+ 走带；
  /// 右（从右往左）= 曲名框、重做、撤销、工具（仅 PR）、量化。
  Widget _buildTopBar() {
    final isPr = _viewMode == kViewPianoroll;
    return Container(
      height: 42,
      color: Color.lerp(YinheTheme.bg, Colors.black, 0.04),
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(
        children: [
          // 左侧：AR = 主页图标；PR = 返回走带（左箭头，与主页返回一致）。
          IconButton(
            onPressed: isPr
                ? () => _setView(kViewArrangement)
                : () => setState(() => _showHome = true),
            icon: Icon(isPr ? Icons.arrow_back : Icons.home),
            color: YinheTheme.text,
            tooltip: isPr ? '返回走带' : '主页',
            visualDensity: VisualDensity.compact,
            padding: EdgeInsets.zero,
            constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
          ),
          IconButton(
            onPressed: _togglePlay,
            icon: Icon(_playing ? Icons.pause : Icons.play_arrow),
            color: YinheTheme.text,
            tooltip: _playing ? '暂停' : '播放',
            visualDensity: VisualDensity.compact,
            padding: EdgeInsets.zero,
            constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
          ),
          IconButton(
            onPressed: _stopPlayback,
            icon: const Icon(Icons.stop),
            color: YinheTheme.textSecondary,
            tooltip: '停止',
            visualDensity: VisualDensity.compact,
            padding: EdgeInsets.zero,
            constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
          ),
          const SizedBox(width: 4),
          // 时间码（无外框、非等宽）：点击切换 秒数 ↔ 小节.拍.tick。
          InkWell(
            borderRadius: BorderRadius.circular(4),
            onTap: () {
              setState(() => _timecodeBar = !_timecodeBar);
              _updateTimecode();
            },
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
              child: ValueListenableBuilder<String>(
                valueListenable: _timecodeText,
                builder: (context, text, _) => Text(
                  text,
                  style: TextStyle(color: YinheTheme.textLabel, fontSize: 14),
                ),
              ),
            ),
          ),
          const Spacer(),
          _buildQuantizeButton(),
          _buildToolButton(),
          ValueListenableBuilder<int>(
            valueListenable: _docState,
            builder: (context, st, _) => Row(
              children: [
                _iconAction(Icons.undo, '撤销', (st & 1) != 0 ? _undo : null),
                _iconAction(Icons.redo, '重做', (st & 2) != 0 ? _redo : null),
              ],
            ),
          ),
          _buildSongBox(),
        ],
      ),
    );
  }

  Widget _iconAction(IconData icon, String tooltip, VoidCallback? onTap) {
    return IconButton(
      onPressed: onTap,
      icon: Icon(icon),
      color: onTap == null ? YinheTheme.textDisabled : YinheTheme.textSecondary,
      tooltip: tooltip,
      visualDensity: VisualDensity.compact,
      padding: EdgeInsets.zero,
      constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
    );
  }

  /// 量化按钮（与桌面同规则：自动/关闭/2 分频链 + 三连音链，只改吸附档位）。
  Widget _buildQuantizeButton() {
    return PopupMenuButton<String>(
      tooltip: '量化（吸附档位）',
      padding: EdgeInsets.zero,
      onSelected: (v) {
        setState(() => _quantizeLabel = v);
        _pushQuantize(v);
      },
      itemBuilder: (_) => [
        const PopupMenuItem(value: '关闭', child: Text('关闭吸附')),
        const PopupMenuItem(value: '自动', child: Text('自动（随缩放）')),
        for (final (label, ticks) in _quantizePresets())
          PopupMenuItem(value: label, child: Text('$label（$ticks tick）')),
      ],
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              Icons.motion_photos_on,
              size: 16,
              color: YinheTheme.textSecondary,
            ),
            const SizedBox(width: 4),
            Text(
              _quantizeLabel,
              style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
            ),
          ],
        ),
      ),
    );
  }

  /// 常用量化档位（与桌面 `presets_for_ppq` 同规则）。
  List<(String, int)> _quantizePresets() {
    final whole = _ppq.round().clamp(1, 1 << 24) * 4;
    final out = <(String, int)>[];
    void chain(int start) {
      var den = start;
      while (den <= whole && whole % den == 0) {
        out.add(('1/$den', whole ~/ den));
        den *= 2;
      }
    }

    chain(1);
    chain(6);
    return out;
  }

  /// 工具选择按钮（PR 5 种；AR 2 种，与 Rust 编号对齐）。
  Widget _buildToolButton() {
    final list = _viewMode == kViewPianoroll ? _toolPrAllowed : _toolArAllowed;
    final current = _tool;
    final shown = list.contains(current) ? current : list.first;
    return PopupMenuButton<int>(
      tooltip: '工具',
      padding: EdgeInsets.zero,
      onSelected: _selectTool,
      itemBuilder: (_) => [
        for (final i in list)
          PopupMenuItem(
            value: i,
            child: Row(
              children: [
                Icon(
                  _toolDefs[i].$1,
                  size: 18,
                  color: i == shown
                      ? YinheTheme.accent
                      : YinheTheme.textSecondary,
                ),
                const SizedBox(width: 8),
                Text(_toolDefs[i].$2),
              ],
            ),
          ),
      ],
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        child: Icon(
          _toolDefs[shown].$1,
          size: 18,
          color: YinheTheme.textSecondary,
        ),
      ),
    );
  }

  /// 曲名框：显示歌曲标题（或当前过滤轨道）；点击更改“显示哪条音符”/导出工程。
  Widget _buildSongBox() {
    final rows = _tracks.value;
    final filtering = _displayTrack >= 0 && _displayTrack < rows.length;
    final base = filtering
        ? rows[_displayTrack].name
        : (_songTitle.isNotEmpty ? _songTitle : (_fileName ?? '未命名'));
    return PopupMenuButton<int>(
      tooltip: '显示轨道 / 保存工程',
      padding: EdgeInsets.zero,
      onSelected: (v) {
        if (v == -2) {
          _saveProject(saveAs: true);
        } else {
          _selectDisplayTrack(v);
        }
      },
      itemBuilder: (_) => [
        const PopupMenuItem(value: -2, child: Text('保存为工程（.ying）…')),
        const PopupMenuDivider(),
        const PopupMenuItem(value: -1, child: Text('全部轨道')),
        for (var i = 0; i < rows.length; i++)
          PopupMenuItem(
            value: i,
            child: Text('${i.toString().padLeft(3, '0')}  ${rows[i].name}'),
          ),
      ],
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 7),
        padding: const EdgeInsets.symmetric(horizontal: 10),
        constraints: const BoxConstraints(maxWidth: 190),
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: YinheTheme.controlBg,
          borderRadius: BorderRadius.circular(6),
        ),
        child: ValueListenableBuilder<int>(
          valueListenable: _docState,
          builder: (context, st, _) => Text(
            (st & 4) != 0 ? '• $base' : base,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              color: filtering ? YinheTheme.accent : YinheTheme.text,
              fontSize: 12,
            ),
          ),
        ),
      ),
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

  /// 工具定义（图标 + 名称；索引与 Rust 编号对齐：0 选框 1 手形 2 铅笔 3 橡皮 4 刷子）。
  static const List<(IconData, String)> _toolDefs = [
    (Icons.highlight_alt, '选择'),
    (Icons.pan_tool, '手形'),
    (Icons.edit, '铅笔'),
    (Icons.auto_fix_normal, '橡皮擦'),
    (Icons.brush, '刷子'),
  ];
}

/// 工程设置对话框（歌名/艺术家/PPQ/简介）。
class _ProjectDialog extends StatefulWidget {
  const _ProjectDialog({
    required this.initialName,
    required this.initialArtist,
    required this.initialDescription,
    required this.initialPpq,
    required this.onSave,
  });

  final String initialName;
  final String initialArtist;
  final String initialDescription;
  final int initialPpq;
  final void Function(String name, String artist, String desc, int ppq) onSave;

  @override
  State<_ProjectDialog> createState() => _ProjectDialogState();
}

class _ProjectDialogState extends State<_ProjectDialog> {
  late final TextEditingController _name = TextEditingController(
    text: widget.initialName,
  );
  late final TextEditingController _artist = TextEditingController(
    text: widget.initialArtist,
  );
  late final TextEditingController _desc = TextEditingController(
    text: widget.initialDescription,
  );
  late final TextEditingController _ppq = TextEditingController(
    text: '${widget.initialPpq}',
  );
  String? _error;

  @override
  void dispose() {
    _name.dispose();
    _artist.dispose();
    _desc.dispose();
    _ppq.dispose();
    super.dispose();
  }

  void _save() {
    final ppq = int.tryParse(_ppq.text.trim());
    if (ppq == null || ppq < 1 || ppq > 32767) {
      setState(() => _error = 'PPQ 需为 1..32767 的整数');
      return;
    }
    widget.onSave(
      _name.text.trim(),
      _artist.text.trim(),
      _desc.text.trim(),
      ppq,
    );
    Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: const Text('工程设置', style: TextStyle(fontSize: 18)),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              _field(_name, '歌名'),
              const SizedBox(height: 12),
              _field(_artist, '艺术家'),
              const SizedBox(height: 12),
              _field(
                _ppq,
                'PPQ（已加载音符时修改会缩放全部时间）',
                keyboardType: TextInputType.number,
              ),
              const SizedBox(height: 12),
              _field(_desc, '简介', maxLines: 3),
              if (_error != null)
                Padding(
                  padding: const EdgeInsets.only(top: 10),
                  child: Text(
                    _error!,
                    style: TextStyle(color: YinheTheme.danger, fontSize: 12),
                  ),
                ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('取消'),
        ),
        FilledButton(onPressed: _save, child: const Text('保存')),
      ],
    );
  }

  Widget _field(
    TextEditingController c,
    String label, {
    int maxLines = 1,
    TextInputType? keyboardType,
  }) {
    return TextField(
      controller: c,
      maxLines: maxLines,
      keyboardType: keyboardType,
      style: const TextStyle(fontSize: 14),
      decoration: InputDecoration(
        labelText: label,
        border: const OutlineInputBorder(),
        isDense: true,
      ),
    );
  }
}
