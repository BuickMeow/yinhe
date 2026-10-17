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
import 'theme.dart';

/// 视图模式（与 Rust 侧约定一致：0=AR 走带，1=PR 卷帘）。
const int kViewArrangement = 0;
const int kViewPianoroll = 1;

void main() {
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
    with SingleTickerProviderStateMixin {
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
  double _seconds = 0;
  double _tick = 0;

  /// 时间码显示模式：false=秒数（默认），true=小节.拍.tick。
  bool _timecodeBar = false;

  List<RecentItem> _recent = const [];

  /// 自动化栏数据（当前轨道，仅 PB/CC/RPN）。
  List<AutomationLaneData> _autoLanes = const [];
  int _autoIndex = 0;

  final ValueNotifier<double> _playhead = ValueNotifier<double>(-1);
  final ValueNotifier<List<double>> _snap =
      ValueNotifier<List<double>>(List<double>.filled(12, 0.0));
  final ValueNotifier<List<TrackRow>> _tracks =
      ValueNotifier<List<TrackRow>>(const []);
  final ValueNotifier<int> _dragFrom = ValueNotifier<int>(-1);
  final ValueNotifier<int> _dragTo = ValueNotifier<int>(-1);

  late final Ticker _ticker;
  Timer? _transportTimer;
  late final ffi.Pointer<ffi.Float> _snapPtr;

  bool _dragging = false;

  @override
  void initState() {
    super.initState();
    _snapPtr = calloc<ffi.Float>(12);
    _channel.setMethodCallHandler(_onNativeCall);
    _init();
    _ticker = createTicker((_) {
      _playhead.value = rust.playheadX();
      _snap.value = _readSnapshot();
    })..start();
    _transportTimer = Timer.periodic(
      const Duration(milliseconds: 250),
      (_) => _refreshTransport(),
    );
    _recent = _readRecent();
  }

  @override
  void dispose() {
    _ticker.dispose();
    _transportTimer?.cancel();
    _playhead.dispose();
    _snap.dispose();
    _tracks.dispose();
    _dragFrom.dispose();
    _dragTo.dispose();
    calloc.free(_snapPtr);
    super.dispose();
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
      _seconds = 0;
      _tick = 0;
      _playing = false;
    });
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
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('设置页尚未实现')),
    );
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

  Color _activeTrackColor() {
    final rows = _tracks.value;
    if (rows.isEmpty) {
      return YinheTheme.accent;
    }
    final i = (_activeTrack ?? 0).clamp(0, rows.length - 1);
    return rows[i].color;
  }

  /// PR 底部自动化栏（高 80 + 1px 分隔线；combo 列 60 宽）。
  Widget _buildAutomationPanel() {
    final lane = _autoLanes.isEmpty
        ? null
        : _autoLanes[_autoIndex.clamp(0, _autoLanes.length - 1).toInt()];
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
              child: PopupMenuButton<int>(
                tooltip: '自动化类型',
                padding: EdgeInsets.zero,
                itemBuilder: (_) => [
                  for (var i = 0; i < _autoLanes.length; i++)
                    PopupMenuItem(value: i, child: Text(_autoLanes[i].name)),
                ],
                onSelected: (i) => setState(() => _autoIndex = i),
                child: Center(
                  child: Text(
                    lane?.name ?? '无自动化',
                    textAlign: TextAlign.center,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: lane == null
                          ? YinheTheme.textDisabled
                          : YinheTheme.textLabel,
                      fontSize: 10,
                    ),
                  ),
                ),
              ),
            ),
            Expanded(
              child: SizedBox.expand(
                child: CustomPaint(
                  painter: AutomationPainter(
                    lane: lane,
                    color: color,
                    snap: _snap,
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  // ── 走带 ──

  void _togglePlay() {
    final next = !_playing;
    setState(() => _playing = next);
    rust.transport(next ? 0 : 1);
  }

  void _stopPlayback() {
    rust.transport(2);
    setState(() {
      _playing = false;
      _seconds = 0;
      _tick = 0;
    });
  }

  void _refreshTransport() {
    if (!mounted || _showHome) {
      return;
    }
    final playing = rust.isPlaying() == 1;
    final seconds = rust.playbackSeconds();
    final tick = rust.playheadTick();
    if (playing != _playing ||
        (seconds - _seconds).abs() > 0.02 ||
        (tick - _tick).abs() > 0.5) {
      setState(() {
        _playing = playing;
        _seconds = seconds;
        _tick = tick;
      });
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
    if (_dragging) {
      return;
    }
    rust.touch(kind, p.dx, p.dy, pointer);
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
        child: LayoutBuilder(
          builder: (context, constraints) {
            final size = constraints.biggest;
            final dpr = MediaQuery.devicePixelRatioOf(context);
            final pixelW = (size.width * dpr).round();
            final pixelH = (size.height * dpr).round();
            if (_reportedPixelSize?.width != pixelW ||
                _reportedPixelSize?.height != pixelH) {
              _reportedPixelSize = Size(pixelW.toDouble(), pixelH.toDouble());
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
                  Positioned.fill(
                    child: IgnorePointer(
                      child: CustomPaint(
                        painter: PlayheadPainter(_playhead),
                      ),
                    ),
                  ),
                if (!_showHome) _buildTopBar(),
                if (!_showHome && _viewMode == kViewPianoroll)
                  _buildAutomationPanel(),
                if (_showHome)
                  HomeView(
                    soundFontName: _soundFontName,
                    recent: _recent,
                    onPickMidi: _pickMidi,
                    onPickSoundFont: _pickSoundFont,
                    onOpenDemo: _openDemo,
                    onOpenRecent: _openRecent,
                    onOpenSettings: _openSettings,
                    onBack: () => setState(() => _showHome = false),
                  ),
              ],
            );
          },
        ),
      ),
    );
  }

  /// 上侧栏：播放/暂停、停止、时间码；右上角主页（PR 时另有走带入口）。
  Widget _buildTopBar() {
    return Positioned(
      left: 0,
      right: 0,
      top: 0,
      child: Container(
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
            // 时间码（无外框）：点击切换 秒数 ↔ 小节.拍.tick。
            InkWell(
              borderRadius: BorderRadius.circular(4),
              onTap: () => setState(() => _timecodeBar = !_timecodeBar),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
                child: Text(
                  _timecodeBar ? _fmtBarBeat(_tick, _ppq) : _fmtTime(_seconds),
                  style: TextStyle(
                    color: YinheTheme.textLabel,
                    fontSize: 14,
                    fontFamily: 'monospace',
                  ),
                ),
              ),
            ),
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
