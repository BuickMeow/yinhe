import '../core/snap_fields.dart';

import 'package:flutter/material.dart';

import '../core/theme.dart';

/// 播放光标：竖线，x 由 Rust 按当前视口换算（逻辑像素）。
class PlayheadPainter extends CustomPainter {
  PlayheadPainter(this.x, this.snap, this.playing, this.activeTrack)
    : super(repaint: Listenable.merge([x, snap]));

  final ValueNotifier<double> x;

  /// 视口快照：`[5]` = 音轨面板/键盘宽度。播放头只允许画在内容区。
  final ValueNotifier<List<double>> snap;

  /// 播放中画满高；暂停时 AR 只画当前选中轨所在行。
  final bool playing;
  final int activeTrack;

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
    var top = 0.0;
    var bottom = size.height;
    if (!playing &&
        s[SnapField.mode] == 0.0 &&
        activeTrack >= 0 &&
        s.length > SnapField.laneHeight) {
      final laneH = s[SnapField.laneHeight];
      final scrollY = s[SnapField.scrollY];
      if (laneH > 0) {
        top = (activeTrack * laneH - scrollY).clamp(0.0, size.height);
        bottom = ((activeTrack + 1) * laneH - scrollY).clamp(top, size.height);
      }
    }
    canvas.drawLine(
      Offset(px, top),
      Offset(px, bottom),
      Paint()
        ..color = YinheTheme.accent
        ..strokeWidth = 1.5,
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => true;
}
