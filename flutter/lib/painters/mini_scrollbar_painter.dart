import 'package:flutter/material.dart';

import '../core/snap_fields.dart';
import '../core/theme.dart';

/// 底部迷你滚动条：只显示整曲长度、当前可见范围和播放头位置，不可拖动。
///
/// 横向坐标与桌面滚动条一致：轨道从内容区（左侧面板之后）开始，
/// 可见范围按 `视口宽 / (总 tick × ppu)` 占比绘制。
class MiniScrollbarPainter extends CustomPainter {
  MiniScrollbarPainter(this.snap, this.playhead)
    : super(repaint: Listenable.merge([snap, playhead]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<double> playhead;

  static const double height = 4.0;

  @override
  void paint(Canvas canvas, Size size) {
    final s = snap.value;
    if (s.length <= SnapField.totalTicks) {
      return;
    }
    final ppu = s[SnapField.ppu];
    final scrollX = s[SnapField.scrollX];
    final total = s[SnapField.totalTicks];
    final panelW = s[SnapField.leftPanelW];
    if (ppu <= 0 || total <= 0) {
      return;
    }
    final contentW = size.width - panelW;
    if (contentW <= 0) {
      return;
    }
    const margin = 6.0;
    final trackLeft = panelW + margin;
    final trackW = contentW - margin * 2;
    if (trackW <= 0) {
      return;
    }
    final y = size.height - height;
    final radius = Radius.circular(height / 2);

    // 轨道底色（整曲长度）。
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(trackLeft, y, trackW, height),
        radius,
      ),
      Paint()..color = YinheTheme.text.withValues(alpha: 0.12),
    );

    final totalPx = total * ppu;
    final visibleFrac = (contentW / totalPx).clamp(0.0, 1.0);
    final startFrac = (scrollX / totalPx).clamp(0.0, 1.0);
    final thumbW = (trackW * visibleFrac).clamp(height * 2, trackW);
    final maxLeft = trackLeft + trackW - thumbW;
    final thumbLeft = (trackLeft + startFrac * trackW).clamp(
      trackLeft,
      maxLeft,
    );
    // 当前视口长度：文本色。
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(thumbLeft, y, thumbW, height),
        radius,
      ),
      Paint()..color = YinheTheme.text.withValues(alpha: 0.55),
    );

    // 播放头位置：按整曲比例映射，而不是直接用屏幕 x 和当前视口重合。
    // 屏幕 x → tick → 整曲比例，与桌面滚动条同一套 scale。
    final playX = playhead.value;
    if (playX.isFinite && playX >= 0) {
      final tick = (playX - panelW + scrollX) / ppu;
      final frac = (tick / total).clamp(0.0, 1.0);
      final px = trackLeft + frac * trackW;
      canvas.drawRect(
        Rect.fromCenter(
          center: Offset(px, y + height / 2),
          width: height,
          height: height,
        ),
        Paint()..color = YinheTheme.accent,
      );
    }
  }

  @override
  bool shouldRepaint(covariant MiniScrollbarPainter oldDelegate) => true;
}
