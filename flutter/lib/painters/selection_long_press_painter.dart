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
      final panelW = s.length > 5 ? s[5] : 0.0;
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
      const r = 18.0;
      canvas.drawCircle(
        p,
        r,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2
          ..color = YinheTheme.accent.withValues(alpha: 0.2),
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

/// 长按圆环走完后的“可框选”提示：一个灰色小方框，放大并淡出。
class MarqueeHintPainter extends CustomPainter {
  MarqueeHintPainter(this.pos, this.progress)
    : super(repaint: Listenable.merge([pos, progress]));

  final ValueNotifier<Offset?> pos;

  /// 0..1（0 = 刚出现，1 = 完全消失）。
  final ValueNotifier<double> progress;

  @override
  void paint(Canvas canvas, Size size) {
    final p = pos.value;
    final t = progress.value.clamp(0.0, 1.0);
    if (p == null || t <= 0 || t >= 1) {
      return;
    }
    // 快速淡入（前 18%），随后放大 + 淡出。
    final fadeIn = (t / 0.18).clamp(0.0, 1.0);
    final fadeOut = 1.0 - ((t - 0.18) / 0.82).clamp(0.0, 1.0);
    final alpha = 0.85 * fadeIn * fadeOut;
    final side = 10.0 + 14.0 * Curves.easeOutCubic.transform(t);
    // 无圆角：和选框锚点一样是方框提示。
    final rect = Rect.fromCenter(center: p, width: side, height: side);
    canvas.drawRect(
      rect,
      Paint()..color = YinheTheme.textSecondary.withValues(alpha: alpha * 0.12),
    );
    canvas.drawRect(
      rect,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.5
        ..color = YinheTheme.textSecondary.withValues(alpha: alpha),
    );
  }

  @override
  bool shouldRepaint(covariant MarqueeHintPainter oldDelegate) => true;
}
