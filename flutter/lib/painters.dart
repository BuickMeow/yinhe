import 'package:flutter/material.dart';

import 'models.dart';
import 'theme.dart';

/// 播放光标：竖线，x 由 Rust 按当前视口换算（逻辑像素）。
class PlayheadPainter extends CustomPainter {
  PlayheadPainter(this.x) : super(repaint: x);

  final ValueNotifier<double> x;

  @override
  void paint(Canvas canvas, Size size) {
    final px = x.value;
    if (px < 0 || px > size.width) {
      return;
    }
    canvas.drawLine(
      Offset(px, 0),
      Offset(px, size.height),
      Paint()
        ..color = YinheTheme.accent
        ..strokeWidth = 1.5,
    );
  }

  @override
  bool shouldRepaint(covariant PlayheadPainter oldDelegate) => false;
}

/// 时间标尺：顶部小节号 + 拍刻度（合并规则与桌面 time_ruler 一致）。
class RulerPainter extends CustomPainter {
  RulerPainter(this.snap) : super(repaint: snap);

  final ValueNotifier<List<double>> snap;

  static const double height = 30.0;
  static const double minLabelSpacing = 38.0;

  @override
  void paint(Canvas canvas, Size size) {
    final s = snap.value;
    if (s.length < 12 || size.width <= 0) {
      return;
    }
    final ppu = s[1];
    final scrollX = s[2];
    final kbW = s[5];
    final ppq = s[8] > 0 ? s[8] : 480.0;
    if (ppu <= 0) {
      return;
    }
    final measureTicks = ppq * 4;
    var divisor = 1.0;
    while (measureTicks * ppu * divisor < minLabelSpacing && divisor < 64) {
      divisor *= 2;
    }
    final barStep = measureTicks * divisor;

    // 背景 + 底线：只覆盖时间区（左侧轨道面板不遮挡）。
    canvas.drawRect(
      Rect.fromLTWH(kbW, 0, size.width - kbW, size.height),
      Paint()..color = YinheTheme.bg,
    );
    canvas.drawLine(
      Offset(kbW, size.height - 0.5),
      Offset(size.width, size.height - 0.5),
      Paint()
        ..color = YinheTheme.lineFg
        ..strokeWidth = 1,
    );

    double xOfTick(double tick) => tick * ppu - scrollX + kbW;
    final startTick = ((-kbW + scrollX) / ppu).clamp(0.0, double.infinity);
    final endTick = ((size.width - kbW + scrollX) / ppu).clamp(
      0.0,
      double.infinity,
    );

    final barPaint = Paint()
      ..color = YinheTheme.textMuted
      ..strokeWidth = 1;
    final beatPaint = Paint()
      ..color = YinheTheme.lineFg
      ..strokeWidth = 1;

    // 拍线（仅在未合并小节标签时）。
    if (divisor == 1.0) {
      var t = (startTick / ppq).floor() * ppq;
      while (t <= endTick + ppq) {
        final x = xOfTick(t);
        if (x >= kbW && x <= size.width && (t % measureTicks != 0)) {
          canvas.drawLine(
            Offset(x, size.height - 7),
            Offset(x, size.height),
            beatPaint,
          );
        }
        t += ppq;
      }
    }

    // 小节刻度 + 小节号。
    var tick = (startTick / barStep).floor() * barStep;
    while (tick <= endTick + barStep) {
      final x = xOfTick(tick);
      if (x >= kbW && x <= size.width) {
        canvas.drawLine(
          Offset(x, size.height - 13),
          Offset(x, size.height),
          barPaint,
        );
        final bar = (tick / measureTicks).floor() + 1;
        _label(canvas, '$bar', Offset(x + 3, 2));
      }
      tick += barStep;
    }
  }

  void _label(Canvas canvas, String text, Offset pos) {
    final tp = TextPainter(
      text: TextSpan(
        text: text,
        style: TextStyle(fontSize: 10, color: YinheTheme.textLabel),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, pos);
  }

  @override
  bool shouldRepaint(covariant RulerPainter oldDelegate) => true;
}

/// 文本浮层：AR 轨道面板文字/编号/M-S、拖动排序指示，PR 键盘 C 音名。
class LabelsPainter extends CustomPainter {
  LabelsPainter(this.snap, this.tracks, this.dragFrom, this.dragTo)
    : super(repaint: Listenable.merge([snap, tracks, dragFrom, dragTo]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<List<TrackRow>> tracks;
  final ValueNotifier<int> dragFrom;
  final ValueNotifier<int> dragTo;

  @override
  void paint(Canvas canvas, Size size) {
    final s = snap.value;
    if (s.length < 12) {
      return;
    }
    if (s[0] == 0.0) {
      _paintArrangement(canvas, size, s, tracks.value);
      _paintDrag(canvas, size, s, dragFrom.value, dragTo.value);
    } else {
      _paintKeyboardLabels(canvas, size, s);
    }
  }

  void _paintArrangement(
    Canvas canvas,
    Size size,
    List<double> s,
    List<TrackRow> rows,
  ) {
    if (rows.isEmpty) {
      return;
    }
    final laneH = s[6];
    final scrollY = s[3];
    final panelW = s[5];
    if (laneH <= 0 || panelW <= 0) {
      return;
    }
    final first = (scrollY / laneH).floor().clamp(0, rows.length).toInt();
    final last = ((scrollY + size.height) / laneH)
        .ceil()
        .clamp(0, rows.length)
        .toInt();
    for (var idx = first; idx < last; idx++) {
      final row = rows[idx];
      final y = idx * laneH - scrollY;
      final port = row.port >= 0 && row.port < 4 ? 'ABCD'[row.port] : '?';
      final code =
          '${idx.toString().padLeft(3, '0')}  $port${(row.channel + 1).toString().padLeft(2, '0')}';
      final font = (laneH * 0.24).clamp(9.0, 13.0).toDouble();
      _text(
        canvas,
        code,
        Offset(10, y + laneH * 0.30),
        font,
        YinheTheme.text,
        vCenter: true,
      );
      final small = (laneH * 0.2).clamp(8.0, 11.0).toDouble();
      _text(
        canvas,
        row.name,
        Offset(10, y + laneH * 0.68),
        small,
        YinheTheme.textLabel,
        vCenter: true,
      );
      if (laneH >= 34) {
        final btn = (laneH * 0.44).clamp(18.0, 26.0).toDouble();
        const gap = 4.0;
        final right = panelW - 6.0;
        final cy = y + laneH / 2;
        _msButton(
          canvas,
          Rect.fromLTWH(right - btn, cy - btn / 2, btn, btn),
          'M',
          row.muted,
          true,
        );
        _msButton(
          canvas,
          Rect.fromLTWH(right - btn * 2 - gap, cy - btn / 2, btn, btn),
          'S',
          row.soloed,
          false,
        );
      }
    }
  }

  /// 拖动排序指示：源行高亮 + 目标行插入线。
  void _paintDrag(Canvas canvas, Size size, List<double> s, int from, int to) {
    if (from < 0 || to < 0) {
      return;
    }
    final laneH = s[6];
    final scrollY = s[3];
    final panelW = s[5];
    if (laneH <= 0 || panelW <= 0) {
      return;
    }
    final fromY = from * laneH - scrollY;
    canvas.drawRect(
      Rect.fromLTWH(0, fromY, panelW, laneH),
      Paint()..color = YinheTheme.accent.withValues(alpha: 0.18),
    );
    final toY = to * laneH - scrollY;
    final lineY = to > from ? toY + laneH : toY;
    canvas.drawRect(
      Rect.fromLTWH(0, lineY - 1, panelW, 2),
      Paint()..color = YinheTheme.accent,
    );
  }

  void _paintKeyboardLabels(Canvas canvas, Size size, List<double> s) {
    final kh = s[4];
    final kbW = s[5];
    final key0Y = s[9];
    if (kh < 10 || kbW <= 0) {
      return;
    }
    final font = (kh * 0.5).clamp(7.0, 11.0).toDouble();
    for (var key = 0; key <= 127; key += 12) {
      final y = key0Y - key * kh;
      if (y + kh < 0 || y > size.height) {
        continue;
      }
      final name = 'C${key ~/ 12 - 1}';
      _text(
        canvas,
        name,
        Offset(kbW - 6, y + kh / 2),
        font,
        YinheTheme.textMuted,
        vCenter: true,
        rightAlign: true,
      );
    }
  }

  void _msButton(
    Canvas canvas,
    Rect rect,
    String label,
    bool active,
    bool isMute,
  ) {
    final bg = active
        ? (isMute ? YinheTheme.warning : YinheTheme.danger)
        : YinheTheme.btnBg;
    final fg = active ? Colors.white : YinheTheme.textMuted;
    canvas.drawRRect(
      RRect.fromRectAndRadius(rect, const Radius.circular(4)),
      Paint()..color = bg,
    );
    _text(canvas, label, rect.center, 10, fg, vCenter: true, hCenter: true);
  }

  void _text(
    Canvas canvas,
    String text,
    Offset pos,
    double fontSize,
    Color color, {
    bool vCenter = false,
    bool hCenter = false,
    bool rightAlign = false,
  }) {
    if (text.isEmpty) {
      return;
    }
    final tp = TextPainter(
      text: TextSpan(
        text: text,
        style: TextStyle(fontSize: fontSize, color: color),
      ),
      maxLines: 1,
      ellipsis: '…',
      textDirection: TextDirection.ltr,
    )..layout();
    var dx = pos.dx;
    if (rightAlign) {
      dx -= tp.width;
    } else if (hCenter) {
      dx -= tp.width / 2;
    }
    final dy = vCenter ? pos.dy - tp.height / 2 : pos.dy;
    tp.paint(canvas, Offset(dx, dy));
  }

  @override
  bool shouldRepaint(covariant LabelsPainter oldDelegate) => false;
}

/// PR 自动化栏曲线（Flutter 自绘：x=tick 变换，y=归一化值 0..1）。

/// PR 自动化栏绘制：力度条或离散（Step）自动化曲线，支持拖动锚点预览。
class AutomationPainter extends CustomPainter {
  AutomationPainter({
    required this.lane,
    required this.velocity,
    required this.color,
    required this.snap,
    required this.preview,
  }) : super(repaint: Listenable.merge([snap, preview]));

  /// null = 力度模式。
  final AutomationLaneData? lane;
  final List<Offset> velocity;
  final Color color;
  final ValueNotifier<List<double>> snap;

  /// 拖动中的锚点预览（tick, value）。
  final ValueNotifier<(double, double)?> preview;

  /// 左侧 combo 列宽（与 main.dart 的 SizedBox 保持一致）。
  static const double comboWidth = 60.0;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.width <= 0 || size.height <= 0) {
      return;
    }
    final s = snap.value;
    if (s.length < 12) {
      return;
    }
    final ppu = s[1];
    final scrollX = s[2];
    final kbW = s[5];
    if (ppu <= 0) {
      return;
    }
    double xOf(double tick) => tick * ppu - scrollX + (kbW - comboWidth);
    double yOf(double value) => size.height * (1.0 - value.clamp(0.0, 1.0));

    final l = lane;
    if (l == null) {
      // 力度条：每个 tick 一根 3px 竖条（底部对齐）。
      final paint = Paint()..color = color;
      for (final p in velocity) {
        final x = xOf(p.dx);
        if (x < -4) {
          continue;
        }
        if (x > size.width + 4) {
          break;
        }
        final top = yOf(p.dy);
        canvas.drawRect(
          Rect.fromLTWH(x - 1.5, top, 3, size.height - top),
          paint,
        );
      }
      return;
    }

    final points = l.points;
    // PB 中心线（归一化 0.5）。
    if (l.kind == 0) {
      final y = size.height * 0.5;
      canvas.drawLine(
        Offset(0, y),
        Offset(size.width, y),
        Paint()..color = YinheTheme.lineFg.withValues(alpha: 0.6),
      );
    }
    if (points.isEmpty) {
      return;
    }

    // 离散（Step）显示：横向保持到下一 tick，再竖直跳变。
    final line = Paint()
      ..color = color.withValues(alpha: 0.85)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;
    final path = Path();
    var started = false;
    var lastY = 0.0;
    for (final p in points) {
      final x = xOf(p.dx);
      final y = yOf(p.dy);
      if (x < -40) {
        lastY = y;
        continue;
      }
      if (x > size.width + 40) {
        break;
      }
      if (!started) {
        path.moveTo(x, y);
        started = true;
      } else {
        path.lineTo(x, lastY);
        path.lineTo(x, y);
      }
      lastY = y;
    }
    if (started) {
      path.lineTo(size.width, lastY);
      canvas.drawPath(path, line);
    }

    // 锚点
    final anchor = Paint()..color = color;
    for (final p in points) {
      final x = xOf(p.dx);
      if (x < 0 || x > size.width) {
        continue;
      }
      canvas.drawRect(
        Rect.fromCenter(center: Offset(x, yOf(p.dy)), width: 5, height: 5),
        anchor,
      );
    }

    // 拖动预览
    final pv = preview.value;
    if (pv != null) {
      final x = xOf(pv.$1);
      final y = yOf(pv.$2);
      canvas.drawLine(
        Offset(x, 0),
        Offset(x, size.height),
        Paint()
          ..color = YinheTheme.accent.withValues(alpha: 0.5)
          ..strokeWidth = 1,
      );
      canvas.drawCircle(
        Offset(x, y),
        6,
        Paint()..color = YinheTheme.accent.withValues(alpha: 0.35),
      );
      canvas.drawRect(
        Rect.fromCenter(center: Offset(x, y), width: 7, height: 7),
        Paint()..color = Colors.white,
      );
    }
  }

  @override
  bool shouldRepaint(covariant AutomationPainter oldDelegate) => true;
}
