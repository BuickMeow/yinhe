import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
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

  /// 力度条数据（tick, velocity01）。
  List<Offset> _velocityBars = const [];

  /// 收起自动化栏（最大化画布）。
  bool _autoCollapsed = false;

  /// 拖动中的锚点预览（tick, value）。
  final ValueNotifier<(double, double)?> _autoPreview = ValueNotifier(null);
  bool _draggingAnchor = false;

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

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _snapPtr = calloc<ffi.Float>(12);
    _channel.setMethodCallHandler(_onNativeCall);
    _init();
    _ticker = createTicker((_) {
      _playhead.value = rust.playheadX();
      _snap.value = _readSnapshot();
      _updateTimecode();
    })..start();
    _recent = _readRecent();
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
    calloc.free(_snapPtr);
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
        });
        rust.setViewMode(kViewArrangement);
        Future.delayed(const Duration(milliseconds: 800), _refreshDocumentMeta);
      case 'onSoundFontPicked':
        setState(() => _soundFontName = path.split('/').last);
    }
  }

  Future<void> _init() async {
    final id = await _channel.invokeMethod<int>('getTextureId');
    if (!mounted) {
      return;
    }
    setState(() => _textureId = id);
    _loadSoundFontName();
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

  void _openDemo() {
    rust.demoLoad();
    rust.setViewMode(kViewArrangement);
    setState(() {
      _fileName = '演示文档';
      _showHome = false;
      _viewMode = kViewArrangement;
      _activeTrack = null;
      _playing = false;
    });
    _updateTimecode();
    Future.delayed(const Duration(milliseconds: 600), _refreshDocumentMeta);
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
    });
    rust.setViewMode(kViewArrangement);
    Future.delayed(const Duration(milliseconds: 800), _refreshDocumentMeta);
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

  /// 读取当前轨道的力度条数据（按 tick 聚合）。
  void _loadVelocity() {
    final rows = _tracks.value;
    if (rows.isEmpty) {
      setState(() => _velocityBars = const []);
      return;
    }
    final track = (_activeTrack ?? 0).clamp(0, rows.length - 1);
    final ptr = rust.velocityJson(track);
    var bars = const <Offset>[];
    if (ptr != ffi.nullptr) {
      try {
        final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
        bars = [
          for (final e in list)
            Offset(
              ((e as List<dynamic>)[0] as num).toDouble(),
              (e[1] as num).toDouble(),
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
    rust.touch(kind, p.dx, p.dy, pointer);
    // 视图快照与演奏指示线同步刷新：缓解 Flutter 浮层与 wgpu 纹理的相位差。
    _snap.value = _readSnapshot();
    _playhead.value = rust.playheadX();
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
                      final dpr = MediaQuery.devicePixelRatioOf(context);
                      final pixelW = (size.width * dpr).round();
                      final pixelH = (size.height * dpr).round();
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
                          'lh': size.height,
                        });
                      }
                      return Stack(
                        children: [
                          Positioned.fill(
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
                            Positioned.fill(
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
                onOpenDemo: _openDemo,
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

  /// 上侧栏：播放/暂停、停止、时间码；右上角主页（PR 时另有走带入口）。
  Widget _buildTopBar() {
    return Container(
      height: 42,
      color: Color.lerp(YinheTheme.bg, Colors.black, 0.04),
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(
        children: [
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
          // 当前文件（标题栏位置）。
          if (_fileName != null)
            Expanded(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 10),
                child: Text(
                  _fileName!,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: YinheTheme.textDisabled,
                    fontSize: 12,
                  ),
                ),
              ),
            )
          else
            const Spacer(),
          if (_viewMode == kViewPianoroll) ...[
            _ViewTab(
              label: '走带',
              selected: false,
              onTap: () => _setView(kViewArrangement),
            ),
            const SizedBox(width: 6),
          ],
          _ViewTab(
            label: '主页',
            selected: false,
            onTap: () => setState(() => _showHome = true),
          ),
        ],
      ),
    );
  }
}

class _ViewTab extends StatelessWidget {
  const _ViewTab({
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: selected ? YinheTheme.accent : YinheTheme.controlBg,
      borderRadius: BorderRadius.circular(14),
      child: InkWell(
        borderRadius: BorderRadius.circular(14),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
          child: Text(
            label,
            style: TextStyle(
              color: selected ? Colors.white : YinheTheme.textSecondary,
              fontSize: 13,
            ),
          ),
        ),
      ),
    );
  }
}
