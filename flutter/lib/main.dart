import 'dart:async';
import 'dart:ffi' as ffi;

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

/// 载入内置演示文档（无文件时的内容源）。
final _VoidDart _demoLoad =
    _bridge.lookupFunction<_VoidNative, _VoidDart>('yinhe_demo_load');

typedef _IntVoidNative = ffi.Void Function(ffi.Int32);
typedef _IntVoidDart = void Function(int);

/// 切换视图：0=AR 走带，1=PR 卷帘。
final _IntVoidDart _setViewMode =
    _bridge.lookupFunction<_IntVoidNative, _IntVoidDart>('yinhe_set_view_mode');

/// 走带控制：0=播放，1=暂停，2=停止。
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
    return const MaterialApp(
      debugShowCheckedModeBanner: false,
      home: StudioShell(),
    );
  }
}

/// 工作台壳：画布常驻（Texture 永不销毁），Home 为覆盖浮层，
/// 顶部条切换 AR/PR 视图，底部为走带控制。
class StudioShell extends StatefulWidget {
  const StudioShell({super.key});

  @override
  State<StudioShell> createState() => _StudioShellState();
}

class _StudioShellState extends State<StudioShell>
    with SingleTickerProviderStateMixin {
  static const MethodChannel _channel = MethodChannel('yinhe/surface');

  int? _textureId;

  /// 最近一次上报的物理像素尺寸（避免 build 里重复调用）。
  Size? _reportedPixelSize;

  bool _showHome = true;
  int _viewMode = kViewArrangement;
  String? _fileName;
  String? _soundFontName;

  bool _playing = false;
  double _seconds = 0;

  /// 播放光标 x（逻辑像素，-1 = 不显示）；Ticker 每帧更新，只重绘光标层。
  final ValueNotifier<double> _playhead = ValueNotifier<double>(-1);
  late final Ticker _ticker;
  Timer? _transportTimer;

  @override
  void initState() {
    super.initState();
    _channel.setMethodCallHandler(_onNativeCall);
    _init();
    _ticker = createTicker((_) => _playhead.value = _playheadX())..start();
    _transportTimer = Timer.periodic(
      const Duration(milliseconds: 250),
      (_) => _refreshTransport(),
    );
  }

  @override
  void dispose() {
    _ticker.dispose();
    _transportTimer?.cancel();
    _playhead.dispose();
    super.dispose();
  }

  /// 低频同步播放状态/时间（播放按键与时间文本）。
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

  /// Kotlin → Dart 事件（文件/音色选择完成）。
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
  }

  /// 触摸事件：坐标为本控件逻辑像素（与 Rust 视图坐标系一致）。
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
        backgroundColor: Color(0xFF14141A),
        body: Center(child: CircularProgressIndicator()),
      );
    }
    return Scaffold(
      backgroundColor: const Color(0xFF14141A),
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
                        painter: _PlayheadPainter(_playhead),
                      ),
                    ),
                  ),
                if (!_showHome) _buildTopBar(),
                if (!_showHome) _buildTransport(),
                if (_showHome)
                  _HomeOverlay(
                    fileName: _fileName,
                    soundFontName: _soundFontName,
                    onPickMidi: _pickMidi,
                    onPickSoundFont: _pickSoundFont,
                    onOpenDemo: _openDemo,
                  ),
              ],
            );
          },
        ),
      ),
    );
  }

  /// 顶部条：AR/PR 切换 + 文件名 + 回主页。
  Widget _buildTopBar() {
    return Positioned(
      left: 0,
      right: 0,
      top: 0,
      child: Container(
        height: 42,
        color: const Color(0xE6101016),
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
                style: const TextStyle(color: Colors.white70, fontSize: 13),
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

  /// 底部走带条：播放/暂停、停止、时间、音色状态。
  Widget _buildTransport() {
    return Positioned(
      left: 0,
      right: 0,
      bottom: 0,
      child: Container(
        height: 48,
        color: const Color(0xE6101016),
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: Row(
          children: [
            IconButton(
              onPressed: _togglePlay,
              icon: Icon(_playing ? Icons.pause : Icons.play_arrow),
              color: Colors.white,
              tooltip: _playing ? '暂停' : '播放',
            ),
            IconButton(
              onPressed: _stopPlayback,
              icon: const Icon(Icons.stop),
              color: Colors.white70,
              tooltip: '停止',
            ),
            const SizedBox(width: 8),
            Text(
              _fmtTime(_seconds),
              style: const TextStyle(
                color: Colors.white70,
                fontSize: 13,
                fontFamily: 'monospace',
              ),
            ),
            const Spacer(),
            Text(
              _soundFontName == null
                  ? '未选择音色库（静音播放）'
                  : '音色：$_soundFontName',
              style: TextStyle(
                color: _soundFontName == null
                    ? Colors.orangeAccent
                    : Colors.white38,
                fontSize: 11,
              ),
            ),
            const SizedBox(width: 8),
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
      color: selected ? const Color(0xFF3A6EA5) : const Color(0xFF23232C),
      borderRadius: BorderRadius.circular(14),
      child: InkWell(
        borderRadius: BorderRadius.circular(14),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
          child: Text(
            label,
            style: TextStyle(
              color: selected ? Colors.white : Colors.white60,
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
      ..color = const Color(0xFFFF5A5A)
      ..strokeWidth = 1.5;
    canvas.drawLine(Offset(px, 0), Offset(px, size.height), paint);
  }

  @override
  bool shouldRepaint(covariant _PlayheadPainter oldDelegate) => false;
}

/// 主页浮层：打开 MIDI / 演示文档 / 选择音色库。
class _HomeOverlay extends StatelessWidget {
  const _HomeOverlay({
    required this.fileName,
    required this.soundFontName,
    required this.onPickMidi,
    required this.onPickSoundFont,
    required this.onOpenDemo,
  });

  final String? fileName;
  final String? soundFontName;
  final VoidCallback onPickMidi;
  final VoidCallback onPickSoundFont;
  final VoidCallback onOpenDemo;

  @override
  Widget build(BuildContext context) {
    return Positioned.fill(
      child: ColoredBox(
        color: const Color(0xF214141A),
        child: Center(
          child: Container(
            width: 440,
            padding: const EdgeInsets.all(24),
            decoration: BoxDecoration(
              color: const Color(0xFF1C1C24),
              borderRadius: BorderRadius.circular(16),
            ),
            child: SingleChildScrollView(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                const Text(
                  '银河 Yinhe',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    color: Colors.white,
                    fontSize: 24,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                const SizedBox(height: 6),
                const Text(
                  '黑乐谱 MIDI 编辑器',
                  textAlign: TextAlign.center,
                  style: TextStyle(color: Colors.white38, fontSize: 13),
                ),
                if (fileName != null) ...[
                  const SizedBox(height: 10),
                  Text(
                    '当前：$fileName',
                    textAlign: TextAlign.center,
                    style:
                        const TextStyle(color: Colors.white54, fontSize: 12),
                  ),
                ],
                const SizedBox(height: 22),
                FilledButton.icon(
                  onPressed: onPickMidi,
                  icon: const Icon(Icons.folder_open),
                  label: const Text('打开 MIDI 文件'),
                ),
                const SizedBox(height: 10),
                OutlinedButton.icon(
                  onPressed: onOpenDemo,
                  icon: const Icon(Icons.music_note),
                  label: const Text('打开演示文档'),
                ),
                const SizedBox(height: 10),
                OutlinedButton.icon(
                  onPressed: onPickSoundFont,
                  icon: const Icon(Icons.piano),
                  label: Text(
                    soundFontName == null
                        ? '选择音色库（SF2）'
                        : '音色：$soundFontName',
                  ),
                ),
                const SizedBox(height: 14),
                const Text(
                  '音色库可选：未选择时播放无声（时钟/光标正常）。\n'
                  '打开后进入走带视图，可切换卷帘；单指拖动 / 双指缩放。',
                  textAlign: TextAlign.center,
                  style: TextStyle(color: Colors.white30, fontSize: 11),
                ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
