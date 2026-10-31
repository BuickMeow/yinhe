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
      _paintHandles(canvas, o.sel.first, stroke, handlesScale.value);
    }
    canvas.restore();
  }

  /// 选区右内侧：`Icons.swap_horiz`（整体 gate）；右下角：L 形角标（Riff）。
  void _paintHandles(Canvas canvas, Rect r, Paint stroke, double scale) {
    if (scale <= 0.01) {
      return;
    }
    final paint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.4
      ..strokeCap = StrokeCap.round
      ..color = YinheTheme.text.withValues(alpha: 0.55);
    // 选区右内侧：标准 swap_horiz 图标（整体调整 gate）。
    final icon = Icons.swap_horiz;
    final iconSize = 14.0 * scale;
    final tp = TextPainter(
      text: TextSpan(
        text: String.fromCharCode(icon.codePoint),
        style: TextStyle(
          fontFamily: icon.fontFamily,
          package: icon.fontPackage,
          fontSize: iconSize,
          color: paint.color,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(
      canvas,
      Offset(r.right - 12.0 - tp.width / 2, r.center.dy - tp.height / 2),
    );

    // 右下角：细 L 形角标（Riff），比加号更像角落手柄。
    final x = r.right - 4.0;
    final y = r.bottom - 4.0;
    final arm = 9.0 * scale;
    canvas.drawPath(
      Path()
        ..moveTo(x - arm, y)
        ..lineTo(x, y)
        ..lineTo(x, y - arm),
      paint,
    );
  }

  @override
  bool shouldRepaint(covariant OverlayPainter oldDelegate) => true;
}
