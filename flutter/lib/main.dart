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

/// 阶段 A 验证页：Rust 渲染的 Texture 画布 + 触摸回传。
class CanvasPage extends StatefulWidget {
  const CanvasPage({super.key});

  @override
  State<CanvasPage> createState() => _CanvasPageState();
}

class _CanvasPageState extends State<CanvasPage> {
  static const MethodChannel _channel = MethodChannel('yinhe/surface');

  int? _textureId;

  /// 最近一次上报给生产端的像素尺寸（避免 build 里重复调用）。
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
    setState(() => _textureId = id);
  }

  /// 逻辑坐标 → NDC（-1..1），与 Rust 侧 `yinhe_touch` 约定一致。
  void _sendTouch(int kind, Offset p, Size size, int pointer) {
    final x = (p.dx / size.width) * 2 - 1;
    final y = 1 - (p.dy / size.height) * 2;
    _touch(kind, x, y, pointer);
  }

  @override
  Widget build(BuildContext context) {
    final id = _textureId;
    if (id == null) {
      return const Scaffold(
        backgroundColor: Color(0xFF0E0E14),
        body: Center(child: CircularProgressIndicator()),
      );
    }
    return Scaffold(
      backgroundColor: const Color(0xFF0E0E14),
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
              _channel.invokeMethod<void>(
                'setSize',
                {'w': pixelW, 'h': pixelH},
              );
            }
            return Listener(
              onPointerDown: (e) => _sendTouch(0, e.localPosition, size, e.pointer),
              onPointerMove: (e) => _sendTouch(1, e.localPosition, size, e.pointer),
              onPointerUp: (e) => _sendTouch(2, e.localPosition, size, e.pointer),
              onPointerCancel: (e) =>
                  _sendTouch(2, e.localPosition, size, e.pointer),
              child: Texture(textureId: id),
            );
          },
        ),
      ),
    );
  }
}
