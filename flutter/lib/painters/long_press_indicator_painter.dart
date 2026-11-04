import '../core/snap_fields.dart';

import 'package:flutter/material.dart';

import '../core/models.dart';
import '../core/theme.dart';

/// 长按落点竖线：
/// - AR：只覆盖长按所在轨道行（高度 = 一个轨道）；
/// - PR：覆盖整个内容区高度（PR 没有行概念）。
///
/// 同一支画笔既用于时间标尺，也用于内容区；内容区里按轨道裁剪。
class LongPressIndicatorPainter extends CustomPainter {
  LongPressIndicatorPainter(this.snap, this.indicator, {this.trackAware = true})
    : super(repaint: Listenable.merge([snap, indicator]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<LongPressIndicator?> indicator;

  /// 是否按 AR 轨道行裁剪（内容区 true；标尺自身画满高度）。
  final bool trackAware;

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
    // 不画到左侧键盘/音轨面板上。
    if (x < origin - 1 || x > size.width + 1) {
      return;
    }
    double y0 = 0;
    double y1 = size.height;
    if (trackAware && !ind.pr) {
      final laneH = s[SnapField.laneHeight];
      final scrollY = s[SnapField.scrollY];
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
