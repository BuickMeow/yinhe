import '../core/snap_fields.dart';

import 'package:flutter/material.dart';

import '../core/theme.dart';

/// 播放光标：竖线，x 由 Rust 按当前视口换算（逻辑像素）。
class PlayheadPainter extends CustomPainter {
  PlayheadPainter(this.x, this.snap)
    : super(repaint: Listenable.merge([x, snap]));

  final ValueNotifier<double> x;

  /// 视口快照：`[5]` = 音轨面板/键盘宽度。播放头只允许画在内容区。
  final ValueNotifier<List<double>> snap;

  @override
  void paint(Canvas canvas, Size size) {
    final px = x.value;
    final s = snap.value;
    final panelW = s.length > 5 ? s[SnapField.leftPanelW] : 0.0;
    if (px < panelW || px > size.width) {
      return;
    }
    canvas.save();
    final contentW = size.width - panelW;
    if (panelW > 0 && contentW > 0) {
      canvas.clipRect(Rect.fromLTWH(panelW, 0, contentW, size.height));
    }
    canvas.drawLine(
      Offset(px, 0),
      Offset(px, size.height),
      Paint()
        ..color = YinheTheme.accent
        ..strokeWidth = 1.5,
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant PlayheadPainter oldDelegate) => false;
}
