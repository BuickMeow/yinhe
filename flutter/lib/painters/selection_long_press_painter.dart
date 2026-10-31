import '../core/snap_fields.dart';

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../core/models.dart';
import '../core/theme.dart';

/// 长按圆环进度 + 移动中的虚线选框。
class SelectionLongPressPainter extends CustomPainter {
  SelectionLongPressPainter(
    this.progress,
    this.pos,
    this.moving,
    this.overlay,
    this.snap,
  ) : super(repaint: Listenable.merge([progress, pos, moving, overlay, snap]));

  final ValueNotifier<double> progress;
  final ValueNotifier<Offset?> pos;
  final ValueNotifier<bool> moving;
  final ValueNotifier<OverlayData> overlay;

  /// 视口快照：`[5]` = 音轨面板/键盘宽度。虚线选框同样只画在内容区。
  final ValueNotifier<List<double>> snap;

  @override
  void paint(Canvas canvas, Size size) {
    if (moving.value) {
      // 移动态只画跟随手指的活动框；原选框已隐藏，避免重复虚线。
      final o = overlay.value;
      final rects = <Rect>[if (o.box != null) o.box!];
      final paint = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = YinheTheme.text.withValues(alpha: 0.45);
      final s = snap.value;
      final panelW = s.length > 5 ? s[SnapField.leftPanelW] : 0.0;
      final contentW = size.width - panelW;
      canvas.save();
      if (panelW > 0 && contentW > 0) {
        canvas.clipRect(Rect.fromLTWH(panelW, 0, contentW, size.height));
      }
      for (final r in rects) {
        _dashedRect(canvas, r, paint);
      }
      canvas.restore();
      return;
    }
    final p = pos.value;
    final v = progress.value;
    if (p != null && v > 0 && v < 1) {
      // 长按进度圆环（-90° 起顺时针）。
      // 手指会挡住圆环中心，半径加大到 28，确保圆环露在指腹外。
      const r = 28.0;
      canvas.drawCircle(
        p,
        r,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 3
          ..color = YinheTheme.accent.withValues(alpha: 0.22),
      );
      canvas.drawArc(
        Rect.fromCircle(center: p, radius: r),
        -math.pi / 2,
        2 * math.pi * v,
        false,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 4
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

/// 长按圆环走完后的“可框选”提示：一个灰色小方框，放大并淡出。
class MarqueeHintPainter extends CustomPainter {
  MarqueeHintPainter(this.pos, this.progress)
    : super(repaint: Listenable.merge([pos, progress]));

  final ValueNotifier<Offset?> pos;

  /// 0..1 = 放大并保持；1..2 = 开始拖框时淡出。
  final ValueNotifier<double> progress;

  @override
  void paint(Canvas canvas, Size size) {
    final p = pos.value;
    final t = progress.value.clamp(0.0, 2.0);
    if (p == null || t <= 0 || t >= 2) {
      return;
    }
    final grow = t.clamp(0.0, 1.0);
    final fadeIn = (grow / 0.18).clamp(0.0, 1.0);
    final fadeOut = t <= 1 ? 1.0 : 1.0 - (t - 1.0).clamp(0.0, 1.0);
    final opacity = fadeIn * fadeOut;
    // 同样避开指腹：从 18 放大到 44。
    final side = 18.0 + 26.0 * Curves.easeOutCubic.transform(grow);
    // 无圆角，填充/描边与正式选框完全一致（OverlayPainter 同款）。
    final rect = Rect.fromCenter(center: p, width: side, height: side);
    canvas.drawRect(
      rect,
      Paint()..color = YinheTheme.text.withValues(alpha: 0.14 * opacity),
    );
    canvas.drawRect(
      rect,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = YinheTheme.text.withValues(alpha: 0.60 * opacity),
    );
  }

  @override
  bool shouldRepaint(covariant MarqueeHintPainter oldDelegate) => true;
}
