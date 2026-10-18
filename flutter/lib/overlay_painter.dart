import 'package:flutter/material.dart';

import 'models.dart';
import 'theme.dart';

/// 选框/预览浮层：持久选框 + 拖动框 + 刷子格（画布内容坐标）。
class OverlayPainter extends CustomPainter {
  OverlayPainter(this.overlay, this.moving)
    : super(repaint: Listenable.merge([overlay, moving]));

  final ValueNotifier<OverlayData> overlay;

  /// 移动态由 SelectionLongPressPainter 画虚线，这里只保留填充。
  final ValueNotifier<bool> moving;

  @override
  void paint(Canvas canvas, Size size) {
    final o = overlay.value;
    final fill = Paint()..color = YinheTheme.text.withValues(alpha: 0.10);
    final stroke = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = YinheTheme.text.withValues(alpha: 0.45);
    void rect(Rect r) {
      canvas.drawRect(r, fill);
      if (!moving.value) {
        canvas.drawRect(r, stroke);
      }
    }

    for (final r in o.sel) {
      rect(r);
    }
    for (final r in o.cells) {
      rect(r);
    }
    final b = o.box;
    if (b != null) {
      rect(b);
    }
  }

  @override
  bool shouldRepaint(covariant OverlayPainter oldDelegate) => true;
}
