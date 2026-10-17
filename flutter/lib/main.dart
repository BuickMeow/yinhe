import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

/// Rust 桥（libyinhe_flutter.so）：触摸走 dart:ffi 同步直连，低开销。
final ffi.DynamicLibrary _bridge =
    ffi.DynamicLibrary.open('libyinhe_flutter.so');

typedef _TouchNative = ffi.Void Function(
    ffi.Int32, ffi.Float, ffi.Float, ffi.Int32);
typedef _TouchDart = void Function(int, double, double, int);

final _TouchDart _touch =
    _bridge.lookupFunction<_TouchNative, _TouchDart>('yinhe_touch');

typedef _VoidNative = ffi.Void Function();
typedef _VoidDart = void Function();

final _VoidDart _demoLoad =
    _bridge.lookupFunction<_VoidNative, _VoidDart>('yinhe_demo_load');

typedef _IntVoidNative = ffi.Void Function(ffi.Int32);
typedef _IntVoidDart = void Function(int);

final _IntVoidDart _setViewMode =
    _bridge.lookupFunction<_IntVoidNative, _IntVoidDart>('yinhe_set_view_mode');
final _IntVoidDart _transport =
    _bridge.lookupFunction<_IntVoidNative, _IntVoidDart>('yinhe_transport');

typedef _IntNative = ffi.Int32 Function();
typedef _IntDart = int Function();

final _IntDart _isPlaying =
    _bridge.lookupFunction<_IntNative, _IntDart>('yinhe_is_playing');

typedef _DoubleNative = ffi.Double Function();
typedef _DoubleDart = double Function();

final _DoubleDart _playbackSeconds =
    _bridge.lookupFunction<_DoubleNative, _DoubleDart>(
        'yinhe_playback_seconds');

typedef _FloatNative = ffi.Float Function();
typedef _FloatDart = double Function();

/// 播放光标 x（Rust 返回 f32，必须按 Float 读）。
final _FloatDart _playheadX =
    _bridge.lookupFunction<_FloatNative, _FloatDart>('yinhe_playhead_x');

typedef _SnapNative = ffi.Int32 Function(ffi.Pointer<ffi.Float>);
typedef _SnapDart = int Function(ffi.Pointer<ffi.Float>);

final _SnapDart _viewSnapshot =
    _bridge.lookupFunction<_SnapNative, _SnapDart>('yinhe_view_snapshot');

typedef _CStrNative = ffi.Pointer<Utf8> Function();
typedef _CStrDart = ffi.Pointer<Utf8> Function();

final _CStrDart _recentJson =
    _bridge.lookupFunction<_CStrNative, _CStrDart>('yinhe_recent_json');
final _CStrDart _tracksJson =
    _bridge.lookupFunction<_CStrNative, _CStrDart>('yinhe_tracks_json');

typedef _FreeNative = ffi.Void Function(ffi.Pointer<Utf8>);
typedef _FreeDart = void Function(ffi.Pointer<Utf8>);

final _FreeDart _freeString =
    _bridge.lookupFunction<_FreeNative, _FreeDart>('yinhe_free_string');

typedef _OpenPathNative = ffi.Void Function(ffi.Pointer<Utf8>);
typedef _OpenPathDart = void Function(ffi.Pointer<Utf8>);

final _OpenPathDart _openPath =
    _bridge.lookupFunction<_OpenPathNative, _OpenPathDart>('yinhe_open_path');

/// 视图模式（与 Rust 侧约定一致：0=AR 走带，1=PR 卷帘）。
const int kViewArrangement = 0;
const int kViewPianoroll = 1;

// ── LIGHT 主题（与 yinhe-theme::base::BaseColors::LIGHT 同步）──

class YinheTheme {
  static const bg = Color(0xFFF0F0F4);
  static const text = Color(0xFF1E1E22);
  static const accent = Color(0xFF1E6EDC);
  static const danger = Color(0xFFE81123);
  static const warning = Color(0xFFF0C83C);

  static const _black = Color(0xFF000000);

  static Color get controlBg => Color.lerp(bg, _black, 0.03) ?? bg;
  static Color get controlSelectedBg => Color.lerp(bg, _black, 0.08) ?? bg;
  static Color get btnBg => Color.lerp(bg, _black, 0.05) ?? bg;
  static Color get lineFg => Color.lerp(bg, _black, 0.07) ?? bg;
  static Color get stripeBg => Color.lerp(bg, _black, 0.06) ?? bg;
  static Color get textSecondary => Color.lerp(text, bg, 0.14) ?? text;
  static Color get textMuted => Color.lerp(text, bg, 0.22) ?? text;
  static Color get textLabel => Color.lerp(text, bg, 0.36) ?? text;
  static Color get textDisabled => Color.lerp(text, bg, 0.66) ?? text;
  static Color get textFaint => Color.lerp(text, bg, 0.9) ?? text;
  static Color get selectedBg => Color.lerp(bg, accent, 0.30) ?? bg;
  static Color get primaryButtonBg => Color.lerp(accent, Colors.white, 0.72) ?? accent;

  static ThemeData data() {
    final scheme = ColorScheme.fromSeed(
      seedColor: accent,
      brightness: Brightness.light,
    ).copyWith(
      primary: accent,
      surface: bg,
    );
    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      scaffoldBackgroundColor: bg,
      splashFactory: InkSparkle.splashFactory,
    );
  }
}

// ── 数据类 ──

class TrackRow {
  const TrackRow({
    required this.name,
    required this.port,
    required this.channel,
    required this.muted,
    required this.soloed,
  });

  final String name;
  final int port;
  final int channel;
  final bool muted;
  final bool soloed;

  static TrackRow fromJson(Map<String, dynamic> json) {
    return TrackRow(
      name: json['name'] as String? ?? '',
      port: (json['port'] as num?)?.toInt() ?? 0,
      channel: (json['channel'] as num?)?.toInt() ?? 0,
      muted: json['muted'] as bool? ?? false,
      soloed: json['soloed'] as bool? ?? false,
    );
  }
}

class RecentItem {
  const RecentItem({required this.name, required this.path, required this.time});

  final String name;
  final String path;
  final int time;

  static RecentItem fromJson(Map<String, dynamic> json) {
    return RecentItem(
      name: json['name'] as String? ?? '',
      path: json['path'] as String? ?? '',
      time: (json['time'] as num?)?.toInt() ?? 0,
    );
  }
}

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

/// 工作台壳：画布常驻，主页为覆盖浮层，顶部切换 AR/PR，底部走带。
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

  bool _playing = false;
  double _seconds = 0;

  List<RecentItem> _recent = const [];

  final ValueNotifier<double> _playhead = ValueNotifier<double>(-1);
  final ValueNotifier<List<double>> _snap =
      ValueNotifier<List<double>>(List<double>.filled(12, 0.0));
  final ValueNotifier<List<TrackRow>> _tracks =
      ValueNotifier<List<TrackRow>>(const []);

  late final Ticker _ticker;
  Timer? _transportTimer;
  late final ffi.Pointer<ffi.Float> _snapPtr;

  @override
  void initState() {
    super.initState();
    _snapPtr = calloc<ffi.Float>(12);
    _channel.setMethodCallHandler(_onNativeCall);
    _init();
    _ticker = createTicker((_) {
      _playhead.value = _playheadX();
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
    calloc.free(_snapPtr);
    super.dispose();
  }

  List<double> _readSnapshot() {
    if (_viewSnapshot(_snapPtr) == 0) {
      return _snap.value;
    }
    return List<double>.generate(12, (i) => _snapPtr[i]);
  }

  List<RecentItem> _readRecent() {
    final ptr = _recentJson();
    if (ptr == ffi.nullptr) {
      return const [];
    }
    try {
      final text = ptr.toDartString();
      final list = jsonDecode(text) as List<dynamic>;
      return [
        for (final e in list) RecentItem.fromJson(e as Map<String, dynamic>),
      ];
    } catch (_) {
      return const [];
    } finally {
      _freeString(ptr);
    }
  }

  List<TrackRow> _readTracks() {
    final ptr = _tracksJson();
    if (ptr == ffi.nullptr) {
      return const [];
    }
    try {
      final text = ptr.toDartString();
      final list = jsonDecode(text) as List<dynamic>;
      return [
        for (final e in list) TrackRow.fromJson(e as Map<String, dynamic>),
      ];
    } catch (_) {
      return const [];
    } finally {
      _freeString(ptr);
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
        });
        _setViewMode(kViewArrangement);
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
    _demoLoad();
    _setViewMode(kViewArrangement);
    setState(() {
      _fileName = '演示文档';
      _showHome = false;
      _viewMode = kViewArrangement;
      _seconds = 0;
      _playing = false;
    });
    Future.delayed(const Duration(milliseconds: 600), _refreshDocumentMeta);
  }

  void _openRecent(RecentItem item) {
    final p = item.path.toNativeUtf8();
    try {
      _openPath(p);
    } finally {
      calloc.free(p);
    }
    setState(() {
      _fileName = item.name;
      _showHome = false;
      _viewMode = kViewArrangement;
    });
    _setViewMode(kViewArrangement);
    Future.delayed(const Duration(milliseconds: 800), _refreshDocumentMeta);
  }

  void _sendTouch(int kind, Offset p, int pointer) {
    _touch(kind, p.dx, p.dy, pointer);
  }

  void _setView(int mode) {
    if (_viewMode == mode) {
      return;
    }
    _setViewMode(mode);
    setState(() => _viewMode = mode);
  }

  void _togglePlay() {
    final next = !_playing;
    setState(() => _playing = next);
    _transport(next ? 0 : 1);
  }

  void _stopPlayback() {
    _transport(2);
    setState(() {
      _playing = false;
      _seconds = 0;
    });
  }

  void _refreshTransport() {
    if (!mounted || _showHome) {
      return;
    }
    final playing = _isPlaying() == 1;
    final seconds = _playbackSeconds();
    if (playing != _playing || (seconds - _seconds).abs() > 0.02) {
      setState(() {
        _playing = playing;
        _seconds = seconds;
      });
    }
  }

  String _fmtTime(double seconds) {
    final total = seconds.floor();
    final m = total ~/ 60;
    final s = total % 60;
    final tenth = ((seconds - total) * 10).floor();
    return '$m:${s.toString().padLeft(2, '0')}.$tenth';
  }

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
                // 画布常驻：切页/开合浮层都不销毁，surface 生命周期稳定。
                Positioned.fill(
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
                if (!_showHome)
                  Positioned.fill(
                    child: IgnorePointer(
                      child: CustomPaint(
                        painter: _LabelsPainter(_snap, _tracks),
                      ),
                    ),
                  ),
                if (!_showHome)
                  Positioned.fill(
                    child: IgnorePointer(
                      child: CustomPaint(
                        painter: _PlayheadPainter(_playhead),
                      ),
                    ),
                  ),
                if (!_showHome) _buildTopBar(),
                if (!_showHome) _buildTransport(),
                if (_showHome) _buildHome(),
              ],
            );
          },
        ),
      ),
    );
  }

  // ── 主页（仿桌面：按钮列 + 最近文件 + 银字水印 + 版本号 + 返回）──

  Widget _buildHome() {
    return Positioned.fill(
      child: ColoredBox(
        color: YinheTheme.bg,
        child: Stack(
          children: [
            // 左下角水印。
            Positioned(
              left: 18,
              bottom: -30,
              child: IgnorePointer(
                child: Text(
                  '银',
                  style: TextStyle(
                    fontSize: 300,
                    height: 1,
                    fontWeight: FontWeight.w700,
                    color: YinheTheme.textFaint,
                  ),
                ),
              ),
            ),
            // 右下角版本号。
            const Positioned(
              right: 16,
              bottom: 12,
              child: Text(
                '0.1.0',
                style: TextStyle(color: Color(0xFF9A9AA2), fontSize: 12),
              ),
            ),
            // 左上角返回。
            Positioned(
              left: 4,
              top: 4,
              child: IconButton(
                tooltip: '返回工作区',
                icon: const Icon(Icons.arrow_back),
                color: YinheTheme.textSecondary,
                onPressed: () => setState(() => _showHome = false),
              ),
            ),
            // 主体：左按钮列 + 右最近文件。
            Positioned.fill(
              child: Padding(
                padding: const EdgeInsets.only(left: 48, right: 40, top: 28),
                child: Row(
                  children: [
                    SizedBox(width: 300, child: _homeActions()),
                    const SizedBox(width: 56),
                    Expanded(child: _recentPanel()),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _homeActions() {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        FilledButton.icon(
          onPressed: _pickMidi,
          icon: const Icon(Icons.note_add, size: 18),
          label: const Text('打开 MIDI 文件'),
          style: FilledButton.styleFrom(
            backgroundColor: YinheTheme.primaryButtonBg,
            foregroundColor: YinheTheme.text,
            minimumSize: const Size.fromHeight(52),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(6),
            ),
          ),
        ),
        const SizedBox(height: 14),
        _secondaryButton('打开演示文档', Icons.folder_open, _openDemo),
        const SizedBox(height: 14),
        _secondaryButton(
          _soundFontName == null
              ? '选择音色库…'
              : '音色库：$_soundFontName',
          Icons.piano,
          _pickSoundFont,
        ),
      ],
    );
  }

  Widget _secondaryButton(String label, IconData icon, VoidCallback onTap) {
    return TextButton.icon(
      onPressed: onTap,
      icon: Icon(icon, size: 18),
      label: Text(label, overflow: TextOverflow.ellipsis),
      style: TextButton.styleFrom(
        backgroundColor: YinheTheme.controlBg,
        foregroundColor: YinheTheme.text,
        minimumSize: const Size.fromHeight(52),
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(6),
        ),
      ),
    );
  }

  Widget _recentPanel() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Text(
          '最近修改的文件',
          style: TextStyle(color: YinheTheme.textLabel, fontSize: 14),
        ),
        const SizedBox(height: 8),
        Expanded(
          child: _recent.isEmpty
              ? Align(
                  alignment: Alignment.topLeft,
                  child: Padding(
                    padding: const EdgeInsets.only(top: 8),
                    child: Text(
                      '暂无最近文件',
                      style: TextStyle(
                        color: YinheTheme.textDisabled,
                        fontSize: 13,
                      ),
                    ),
                  ),
                )
              : ListView.builder(
                  padding: EdgeInsets.zero,
                  itemCount: _recent.length,
                  itemBuilder: (context, i) {
                    final item = _recent[i];
                    return InkWell(
                      onTap: () => _openRecent(item),
                      child: Padding(
                        padding: const EdgeInsets.symmetric(vertical: 7),
                        child: Row(
                          children: [
                            Icon(
                              Icons.audio_file,
                              size: 18,
                              color: YinheTheme.textLabel,
                            ),
                            const SizedBox(width: 8),
                            Expanded(
                              child: Text(
                                item.name,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: TextStyle(
                                  color: YinheTheme.text,
                                  fontSize: 14,
                                ),
                              ),
                            ),
                            Text(
                              _fmtDate(item.time),
                              style: TextStyle(
                                color: YinheTheme.textDisabled,
                                fontSize: 12,
                              ),
                            ),
                          ],
                        ),
                      ),
                    );
                  },
                ),
        ),
      ],
    );
  }

  String _fmtDate(int epochSeconds) {
    if (epochSeconds <= 0) {
      return '';
    }
    final dt = DateTime.fromMillisecondsSinceEpoch(epochSeconds * 1000);
    String two(int v) => v.toString().padLeft(2, '0');
    return '${dt.year}-${two(dt.month)}-${two(dt.day)} '
        '${two(dt.hour)}:${two(dt.minute)}';
  }

  // ── 顶部条 / 底部走带 ──

  Widget _buildTopBar() {
    return Positioned(
      left: 0,
      right: 0,
      top: 0,
      child: Container(
        height: 42,
        color: Color.lerp(YinheTheme.bg, Colors.black, 0.04),
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
        child: Row(
          children: [
            _ViewTab(
              label: '走带 AR',
              selected: _viewMode == kViewArrangement,
              onTap: () => _setView(kViewArrangement),
            ),
            const SizedBox(width: 6),
            _ViewTab(
              label: '卷帘 PR',
              selected: _viewMode == kViewPianoroll,
              onTap: () => _setView(kViewPianoroll),
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Text(
                _fileName ?? '',
                style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
                overflow: TextOverflow.ellipsis,
              ),
            ),
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

  Widget _buildTransport() {
    return Positioned(
      left: 0,
      right: 0,
      bottom: 0,
      child: Container(
        height: 48,
        color: Color.lerp(YinheTheme.bg, Colors.black, 0.04),
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: Row(
          children: [
            IconButton(
              onPressed: _togglePlay,
              icon: Icon(_playing ? Icons.pause : Icons.play_arrow),
              color: YinheTheme.text,
              tooltip: _playing ? '暂停' : '播放',
            ),
            IconButton(
              onPressed: _stopPlayback,
              icon: const Icon(Icons.stop),
              color: YinheTheme.textSecondary,
              tooltip: '停止',
            ),
            const SizedBox(width: 8),
            Text(
              _fmtTime(_seconds),
              style: TextStyle(
                color: YinheTheme.textLabel,
                fontSize: 13,
                fontFamily: 'monospace',
              ),
            ),
            const Spacer(),
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

/// 播放光标：竖线，x 由 Rust 按当前视口换算（逻辑像素）。
class _PlayheadPainter extends CustomPainter {
  _PlayheadPainter(this.x) : super(repaint: x);

  final ValueNotifier<double> x;

  @override
  void paint(Canvas canvas, Size size) {
    final px = x.value;
    if (px < 0 || px > size.width) {
      return;
    }
    final paint = Paint()
      ..color = YinheTheme.danger
      ..strokeWidth = 1.5;
    canvas.drawLine(Offset(px, 0), Offset(px, size.height), paint);
  }

  @override
  bool shouldRepaint(covariant _PlayheadPainter oldDelegate) => false;
}

/// 文本浮层：AR 轨道面板文字/编号/M-S，PR 键盘 C 音名。
class _LabelsPainter extends CustomPainter {
  _LabelsPainter(this.snap, this.tracks)
      : super(repaint: Listenable.merge([snap, tracks]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<List<TrackRow>> tracks;

  @override
  void paint(Canvas canvas, Size size) {
    final s = snap.value;
    if (s.length < 12) {
      return;
    }
    if (s[0] == 0.0) {
      _paintArrangement(canvas, size, s, tracks.value);
    } else {
      _paintKeyboardLabels(canvas, size, s);
    }
  }

  void _paintArrangement(
    Canvas canvas,
    Size size,
    List<double> s,
    List<TrackRow> rows,
  ) {
    if (rows.isEmpty) {
      return;
    }
    final laneH = s[6];
    final scrollY = s[3];
    final panelW = s[5];
    if (laneH <= 0 || panelW <= 0) {
      return;
    }
    final first =
        (scrollY / laneH).floor().clamp(0, rows.length).toInt();
    final last =
        ((scrollY + size.height) / laneH).ceil().clamp(0, rows.length).toInt();
    for (var idx = first; idx < last; idx++) {
      final row = rows[idx];
      final y = idx * laneH - scrollY;
      final port = row.port >= 0 && row.port < 4
          ? 'ABCD'[row.port]
          : '?';
      final code =
          '${idx.toString().padLeft(3, '0')}  $port${(row.channel + 1).toString().padLeft(2, '0')}';
      final font = (laneH * 0.24).clamp(9.0, 13.0).toDouble();
      _text(
        canvas,
        code,
        Offset(10, y + laneH * 0.30),
        font,
        YinheTheme.text,
        vCenter: true,
      );
      final small = (laneH * 0.2).clamp(8.0, 11.0).toDouble();
      _text(
        canvas,
        row.name,
        Offset(10, y + laneH * 0.68),
        small,
        YinheTheme.textLabel,
        vCenter: true,
      );
      if (laneH >= 34) {
        final btn = (laneH * 0.44).clamp(18.0, 26.0).toDouble();
        const gap = 4.0;
        final right = panelW - 6.0;
        final cy = y + laneH / 2;
        _msButton(
          canvas,
          Rect.fromLTWH(right - btn, cy - btn / 2, btn, btn),
          'M',
          row.muted,
          true,
        );
        _msButton(
          canvas,
          Rect.fromLTWH(right - btn * 2 - gap, cy - btn / 2, btn, btn),
          'S',
          row.soloed,
          false,
        );
      }
    }
  }

  void _paintKeyboardLabels(Canvas canvas, Size size, List<double> s) {
    final kh = s[4];
    final kbW = s[5];
    final key0Y = s[9];
    if (kh < 10 || kbW <= 0) {
      return;
    }
    final font = (kh * 0.5).clamp(7.0, 11.0).toDouble();
    for (var key = 0; key <= 127; key += 12) {
      final y = key0Y - key * kh;
      if (y + kh < 0 || y > size.height) {
        continue;
      }
      final name = 'C${key ~/ 12 - 1}';
      _text(
        canvas,
        name,
        Offset(kbW - 6, y + kh / 2),
        font,
        YinheTheme.textMuted,
        vCenter: true,
        rightAlign: true,
      );
    }
  }

  void _msButton(
    Canvas canvas,
    Rect rect,
    String label,
    bool active,
    bool isMute,
  ) {
    final bg = active
        ? (isMute ? YinheTheme.warning : YinheTheme.danger)
        : YinheTheme.btnBg;
    final fg = active ? Colors.white : YinheTheme.textMuted;
    canvas.drawRRect(
      RRect.fromRectAndRadius(rect, const Radius.circular(4)),
      Paint()..color = bg,
    );
    _text(
      canvas,
      label,
      rect.center,
      10,
      fg,
      vCenter: true,
      hCenter: true,
    );
  }

  void _text(
    Canvas canvas,
    String text,
    Offset pos,
    double fontSize,
    Color color, {
    bool vCenter = false,
    bool hCenter = false,
    bool rightAlign = false,
  }) {
    if (text.isEmpty) {
      return;
    }
    final tp = TextPainter(
      text: TextSpan(
        text: text,
        style: TextStyle(fontSize: fontSize, color: color),
      ),
      maxLines: 1,
      ellipsis: '…',
      textDirection: TextDirection.ltr,
    )..layout();
    var dx = pos.dx;
    if (rightAlign) {
      dx -= tp.width;
    } else if (hCenter) {
      dx -= tp.width / 2;
    }
    final dy = vCenter ? pos.dy - tp.height / 2 : pos.dy;
    tp.paint(canvas, Offset(dx, dy));
  }

  @override
  bool shouldRepaint(covariant _LabelsPainter oldDelegate) => false;
}
