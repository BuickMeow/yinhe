import '../core/snap_fields.dart';

import 'package:flutter/material.dart';

import '../core/models.dart';
import '../core/theme.dart';

/// 时间标尺上的插入落点竖线：只覆盖标尺自身的高度。
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
    final ppu = s[SnapField.ppu];
    final scrollX = s[SnapField.scrollX];
    final origin = s[SnapField.leftPanelW];
    if (ppu <= 0) {
      return;
    }
    final x = ind.tick * ppu - scrollX + origin;
    if (x < -1 || x > size.width + 1) {
      return;
    }
    canvas.drawRect(
      Rect.fromLTRB(x - 1, 0, x + 1, size.height),
      Paint()..color = YinheTheme.accent.withValues(alpha: 0.8),
    );
  }

  @override
  bool shouldRepaint(covariant LongPressIndicatorPainter oldDelegate) => true;
}
