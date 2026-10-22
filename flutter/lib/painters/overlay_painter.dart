import 'package:flutter/material.dart';

import '../core/models.dart';
import '../core/theme.dart';

/// 选框/预览浮层：持久选框 + 拖动框 + 刷子格（画布内容坐标）。
class OverlayPainter extends CustomPainter {
  OverlayPainter(this.overlay, this.moving, this.snap, this.showHandles)
    : super(repaint: Listenable.merge([overlay, moving, snap]));

  final ValueNotifier<OverlayData> overlay;

  /// 移动态由 SelectionLongPressPainter 画虚线，这里只保留填充。
  final ValueNotifier<bool> moving;

  /// 视口快照：`[5]` = 音轨面板/键盘宽度。选框只允许画在内容区。
  final ValueNotifier<List<double>> snap;

  /// 当前是选框工具且没有拖动时才显示选区控制点。
  final bool showHandles;

  @override
  void paint(Canvas canvas, Size size) {
    final o = overlay.value;
    final s = snap.value;
    final panelW = s.length > 5 ? s[5] : 0.0;
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
    if (b != null && !moving.value) {
      rect(b);
    }
    canvas.restore();

    if (showHandles &&
        !moving.value &&
        o.box == null &&
        o.sel.length == 1) {
      _paintHandles(canvas, o.sel.first, stroke);
    }
  }

  /// 四个边中点圆形控制点 + 右下角方形 Riff 控制点。
  void _paintHandles(Canvas canvas, Rect r, Paint stroke) {
    final fill = Paint()..color = YinheTheme.bg;
    final handle = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5
      ..color = stroke.color;
    final centers = <Offset>[
      Offset(r.center.dx, r.top),
      Offset(r.center.dx, r.bottom),
      Offset(r.left, r.center.dy),
      Offset(r.right, r.center.dy),
    ];
    for (final c in centers) {
      canvas.drawCircle(c, 4.5, fill);
      canvas.drawCircle(c, 4.5, handle);
    }
    final riff = Rect.fromCenter(center: r.bottomRight, width: 9, height: 9);
    canvas.drawRect(riff, fill);
    canvas.drawRect(riff, handle);
  }

  @override
  bool shouldRepaint(covariant OverlayPainter oldDelegate) => true;
}
