import 'package:flutter/material.dart';

import '../core/theme.dart';

/// Rust 算好的时间网格线：tick + 层级。
///
/// 0=小节、1=拍、2=细分、3=tick（透明度依次降低）。
class GridLine {
  const GridLine(this.tick, this.level);

  final int tick;
  final int level;

  Color color(Color line) => switch (level) {
    0 => line,
    1 => line.withValues(alpha: 140 / 255),
    2 => line.withValues(alpha: 110 / 255),
    _ => line.withValues(alpha: 60 / 255),
  };
}

/// 画竖向时间网格：拍号分段/密度由 Rust 决策，这里只做 x 换算与绘制。
///
/// [leftInset] 是时间轴原点在画布内的 x：
/// PR 用键盘宽度，AR 用轨道面板宽度，AM 用 `键盘宽度 - combo 列宽`。
void paintTimeGrid(
  Canvas canvas, {
  required Size size,
  required double ppu,
  required double scrollX,
  required double leftInset,
  required List<GridLine> lines,
  bool withSubTick = true,
}) {
  if (ppu <= 0 || size.width <= 0 || size.height <= 0) {
    return;
  }
  final paint = Paint()..strokeWidth = 1.0;
  final line = YinheTheme.lineFg;
  for (final g in lines) {
    if (!withSubTick && g.level >= 2) {
      continue;
    }
    final x = g.tick * ppu - scrollX + leftInset;
    if (x < 0 || x > size.width) {
      continue;
    }
    paint.color = g.color(line);
    canvas.drawLine(Offset(x, 0), Offset(x, size.height), paint);
  }
}
