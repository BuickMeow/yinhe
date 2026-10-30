import 'package:flutter/material.dart';

import '../core/theme.dart';

/// 标尺上的一个标签（位置/文字/层级由 Rust 计算）。
class RulerLabel {
  const RulerLabel(this.x, this.text, this.level);

  /// 画布逻辑像素 x（已含键盘/面板偏移）。
  final double x;
  final String text;

  /// 0=小节、1=拍、2=拍内 tick、3=子拍。
  final int level;

  Color get color => switch (level) {
    0 => YinheTheme.measureLabel,
    1 => YinheTheme.textLabel,
    2 => YinheTheme.tickLabel,
    _ => YinheTheme.textDisabled,
  };
}

/// 时间标尺：背景 + Rust 算好的标签，无刻度线（与桌面 `time_ruler` 一致）。
///
/// 拍号分段、小节号、标签密度全部在 Rust（`scene::ruler_labels_json`），
/// Dart 只负责画文字。
class RulerPainter extends CustomPainter {
  RulerPainter(this.labels) : super(repaint: labels);

  final ValueNotifier<List<RulerLabel>> labels;

  /// 标尺高度（逻辑像素，与桌面 `RULER_H` 一致）。
  static const double height = 30.0;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(
      Rect.fromLTWH(0, 0, size.width, size.height),
      Paint()..color = YinheTheme.trackBg,
    );
    final list = labels.value;
    if (list.isEmpty) {
      return;
    }
    final yCenter = size.height * 0.72;
    for (final label in list) {
      if (label.x > size.width + 40) {
        continue;
      }
      _label(canvas, label.text, Offset(label.x + 2, yCenter), label.color);
    }
  }

  void _label(Canvas canvas, String text, Offset pos, Color color) {
    final tp = TextPainter(
      text: TextSpan(
        text: text,
        style: TextStyle(fontSize: 10, color: color),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, Offset(pos.dx, pos.dy - tp.height / 2));
  }

  @override
  bool shouldRepaint(covariant RulerPainter oldDelegate) => true;
}
