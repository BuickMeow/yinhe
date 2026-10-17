import 'dart:ffi' as ffi;

import 'package:flutter/material.dart';
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

/// 载入内置演示文档（阶段 B 临时入口；SAF 文件打开接入后移除）。
final _VoidDart _demoLoad =
    _bridge.lookupFunction<_VoidNative, _VoidDart>('yinhe_demo_load');

void main() {
  runApp(const YinheApp());
}

class YinheApp extends StatelessWidget {
  const YinheApp({super.key});

  @override
  Widget build(BuildContext context) {
    return const MaterialApp(
      debugShowCheckedModeBanner: false,
      home: CanvasPage(),
    );
  }
}

/// 阶段 B 验证页：wgpu 渲染的钢琴卷帘画布 + 触摸手势。
class CanvasPage extends StatefulWidget {
  const CanvasPage({super.key});

  @override
  State<CanvasPage> createState() => _CanvasPageState();
}

class _CanvasPageState extends State<CanvasPage> {
  static const MethodChannel _channel = MethodChannel('yinhe/surface');

  int? _textureId;

  /// 最近一次上报的物理像素尺寸（避免 build 里重复调用）。
  Size? _reportedPixelSize;

  @override
  void initState() {
    super.initState();
    _init();
  }

  Future<void> _init() async {
    final id = await _channel.invokeMethod<int>('getTextureId');
    if (!mounted) {
      return;
    }
    _demoLoad();
    setState(() => _textureId = id);
  }

  /// 触摸事件：坐标为本控件逻辑像素（与 Rust 视图坐标系一致）。
  void _sendTouch(int kind, Offset p, int pointer) {
    _touch(kind, p.dx, p.dy, pointer);
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
                const Positioned(left: 12, top: 8, child: _HintChip()),
              ],
            );
          },
        ),
      ),
    );
  }
}

class _HintChip extends StatelessWidget {
  const _HintChip();

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
      decoration: BoxDecoration(
        color: Colors.black.withValues(alpha: 0.45),
        borderRadius: BorderRadius.circular(14),
      ),
      child: const Text(
        '演示文档 · 单指拖动 / 双指缩放',
        style: TextStyle(color: Colors.white70, fontSize: 12),
      ),
    );
  }
}
