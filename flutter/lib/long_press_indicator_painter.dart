import 'package:flutter/material.dart';

import 'models.dart';
import 'theme.dart';

/// 长按空白处时显示的落点竖线：
/// - AR：只覆盖长按所在轨道行（高度 = 一个轨道）；
/// - PR：覆盖整个内容区高度（PR 没有行概念）。
class LongPressIndicatorPainter extends CustomPainter {
  LongPressIndicatorPainter(this.snap, this.indicator)
    : super(repaint: Listenable.merge([snap, indicator]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<LongPressIndicator?> indicator;

  @override
  void paint(Canvas canvas, Size size) {
    final ind = indicator.value;
    if (ind == null) {
      return;
    }
    final s = snap.value;
    if (s.length < 12 || size.height <= 0) {
      return;
    }
    final ppu = s[1];
    final scrollX = s[2];
    final origin = s[5];
    if (ppu <= 0) {
      return;
    }
    final x = ind.tick * ppu - scrollX + origin;
    double y0 = 0;
    double y1 = size.height;
    if (!ind.pr) {
      final laneH = s[6];
      final scrollY = s[3];
      y0 = ind.track * laneH - scrollY;
      y1 = y0 + laneH;
    }
    y0 = y0.clamp(0.0, size.height);
    y1 = y1.clamp(0.0, size.height);
    if (y1 <= y0) {
      return;
    }
    canvas.drawRect(
      Rect.fromLTRB(x - 1, y0, x + 1, y1),
      Paint()..color = YinheTheme.accent.withValues(alpha: 0.8),
    );
  }

  @override
  bool shouldRepaint(covariant LongPressIndicatorPainter oldDelegate) => true;
}
