import 'package:flutter/material.dart';

import '../core/models.dart';
import '../core/theme.dart';

/// 选框/预览浮层：持久选框 + 拖动框 + 刷子格（画布内容坐标）。
class OverlayPainter extends CustomPainter {
  OverlayPainter(this.overlay, this.moving, this.snap)
    : super(repaint: Listenable.merge([overlay, moving, snap]));

  final ValueNotifier<OverlayData> overlay;

  /// 移动态由 SelectionLongPressPainter 画虚线，这里只保留填充。
  final ValueNotifier<bool> moving;

  /// 视口快照：`[5]` = 音轨面板/键盘宽度。选框只允许画在内容区。
  final ValueNotifier<List<double>> snap;

  @override
  void paint(Canvas canvas, Size size) {
    final o = overlay.value;
    final s = snap.value;
    final panelW = s.length > 5 ? s[5] : 0.0;
    final contentW = size.width - panelW;
    canvas.save();
    if (panelW > 0 && contentW > 0) {
      canvas.clipRect(Rect.fromLTWH(panelW, 0, contentW, size.height));
    }
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
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant OverlayPainter oldDelegate) => true;
}
