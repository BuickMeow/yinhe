import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'models.dart';
import 'theme.dart';

/// 长按圆环进度 + 移动中的虚线选框。
class SelectionLongPressPainter extends CustomPainter {
  SelectionLongPressPainter(this.progress, this.pos, this.moving, this.overlay)
    : super(repaint: Listenable.merge([progress, pos, moving, overlay]));

  final ValueNotifier<double> progress;
  final ValueNotifier<Offset?> pos;
  final ValueNotifier<bool> moving;
  final ValueNotifier<OverlayData> overlay;

  @override
  void paint(Canvas canvas, Size size) {
    if (moving.value) {
      // 移动态：沿用选框原本的字体色描边，只把它变成虚线（不额外加强调色）。
      final o = overlay.value;
      final rects = <Rect>[...o.sel, if (o.box != null) o.box!];
      final paint = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = YinheTheme.text.withValues(alpha: 0.45);
      for (final r in rects) {
        _dashedRect(canvas, r, paint);
      }
      return;
    }
    final p = pos.value;
    final v = progress.value;
    if (p != null && v > 0 && v < 1) {
      // 长按进度圆环（-90° 起顺时针）。
      const r = 18.0;
      canvas.drawCircle(
        p,
        r,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2
          ..color = YinheTheme.text.withValues(alpha: 0.2),
      );
      canvas.drawArc(
        Rect.fromCircle(center: p, radius: r),
        -math.pi / 2,
        2 * math.pi * v,
        false,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 3
          ..strokeCap = StrokeCap.round
          ..color = YinheTheme.accent,
      );
      return;
    }
  }

  void _dashedRect(Canvas canvas, Rect rect, Paint paint) {
    final path = Path()..addRect(rect);
    for (final metric in path.computeMetrics()) {
      var d = 0.0;
      while (d < metric.length) {
        final end = math.min(d + 6.0, metric.length);
        canvas.drawPath(metric.extractPath(d, end), paint);
        d += 10.0;
      }
    }
  }

  @override
  bool shouldRepaint(covariant SelectionLongPressPainter oldDelegate) => true;
}
