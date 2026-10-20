import 'package:flutter/material.dart';

import '../core/theme.dart';

/// 播放光标：竖线，x 由 Rust 按当前视口换算（逻辑像素）。
class PlayheadPainter extends CustomPainter {
  PlayheadPainter(this.x) : super(repaint: x);

  final ValueNotifier<double> x;

  @override
  void paint(Canvas canvas, Size size) {
    final px = x.value;
    if (px < 0 || px > size.width) {
      return;
    }
    canvas.drawLine(
      Offset(px, 0),
      Offset(px, size.height),
      Paint()
        ..color = YinheTheme.accent
        ..strokeWidth = 1.5,
    );
  }

  @override
  bool shouldRepaint(covariant PlayheadPainter oldDelegate) => false;
}
