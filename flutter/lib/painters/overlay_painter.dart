import '../core/snap_fields.dart';

import 'package:flutter/material.dart';

import '../core/models.dart';
import '../core/theme.dart';

/// 选框/预览浮层：持久选框 + 拖动框 + 刷子格（画布内容坐标）。
class OverlayPainter extends CustomPainter {
  OverlayPainter(
    this.overlay,
    this.moving,
    this.snap,
    this.showHandles,
    this.handlesScale,
    this.quantizeTicks,
  ) : super(repaint: Listenable.merge([overlay, moving, snap, handlesScale]));

  final ValueNotifier<OverlayData> overlay;

  /// 移动态由 SelectionLongPressPainter 画虚线，这里只保留填充。
  final ValueNotifier<bool> moving;

  /// 视口快照：`[5]` = 音轨面板/键盘宽度。选框只允许画在内容区。
  final ValueNotifier<List<double>> snap;

  /// 当前是选框工具且没有拖动时才显示选区控制点。
  final bool showHandles;

  /// 锚点出现动画：0 = 隐藏，1 = 正常大小（可略大于 1 做回弹）。
  final ValueNotifier<double> handlesScale;

  /// 当前量化间隔（tick）：右侧 gate 方块从右缘向内缩这么多。
  final double quantizeTicks;

  @override
  void paint(Canvas canvas, Size size) {
    final o = overlay.value;
    final s = snap.value;
    final panelW = s.length > 5 ? s[SnapField.leftPanelW] : 0.0;
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

    // 移动选区时只显示跟随手指的活动框；原选框不再重复绘制。
    if (!moving.value) {
      for (final r in o.sel) {
        rect(r);
      }
    }
    for (final r in o.cells) {
      rect(r);
    }
    final b = o.box;
    if (b != null) {
      // 移动态由 SelectionLongPressPainter 画虚线；这里仍画同一填充。
      rect(b);
    }
    if (showHandles && !moving.value && o.box == null && o.sel.length == 1) {
      _paintHandles(canvas, o.sel.first, handlesScale.value);
    }
    canvas.restore();
  }

  /// 三个方块控制点：
  /// - 右下角：重复（Riff）；
  /// - 右缘中点：整体变速；
  /// - 右缘内缩一个量化格：每个音符批量改 gate（仅 PR）。
  void _paintHandles(Canvas canvas, Rect r, double scale) {
    if (scale <= 0.01) {
      return;
    }
    final s = snap.value;
    final ppu = s.length > SnapField.ppu ? s[SnapField.ppu] : 0.0;
    final isPr = s.length > SnapField.mode && s[SnapField.mode] > 0;
    final fill = Paint()
      ..style = PaintingStyle.fill
      ..color = YinheTheme.text.withValues(alpha: 0.62);
    final border = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = YinheTheme.bg.withValues(alpha: 0.9);
    final side = 8.0 * scale;
    void square(Offset center) {
      final rect = Rect.fromCenter(center: center, width: side, height: side);
      canvas.drawRect(rect, fill);
      canvas.drawRect(rect, border);
    }

    const inset = 4.0;
    // 右下角：重复。
    square(Offset(r.right - inset, r.bottom - inset));
    // 右缘中点：整体变速。
    square(Offset(r.right - inset, r.center.dy));
    // 量化格内缩：批量 gate（AR 没有）。
    if (isPr && ppu > 0 && quantizeTicks > 0) {
      final x = r.right - inset - quantizeTicks * ppu;
      if (x > r.left + 6.0) {
        square(Offset(x, r.center.dy));
      }
    }
  }

  @override
  bool shouldRepaint(covariant OverlayPainter oldDelegate) => true;
}
