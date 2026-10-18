import 'package:flutter/material.dart';

import 'models.dart';
import 'theme.dart';

/// 选框/预览浮层：持久选框 + 拖动框 + 刷子格（画布内容坐标）。
class OverlayPainter extends CustomPainter {
  OverlayPainter(this.overlay) : super(repaint: overlay);

  final ValueNotifier<OverlayData> overlay;

  @override
  void paint(Canvas canvas, Size size) {
    final o = overlay.value;
    final fill = Paint()..color = YinheTheme.text.withValues(alpha: 0.10);
    final stroke = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = YinheTheme.text.withValues(alpha: 0.45);
    void rect(Rect r) {
      canvas.drawRect(r, fill);
      canvas.drawRect(r, stroke);
    }

    for (final r in o.sel) {
      rect(r);
    }
    for (final r in o.cells) {
      rect(r);
    }
    final b = o.box;
    if (b != null) {
      rect(b);
    }
  }

  @override
  bool shouldRepaint(covariant OverlayPainter oldDelegate) => true;
}

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

/// 时间标尺（与桌面 `time_ruler` 一致）：只有小节/拍/子拍/tick 文本标签，
/// 无刻度线；背景铺 `track_bg`（含左侧面板/键盘上方留白）。
class RulerPainter extends CustomPainter {
  RulerPainter(this.snap) : super(repaint: snap);

  final ValueNotifier<List<double>> snap;

  /// 标尺高度（逻辑像素，与桌面 `RULER_H` 一致）。
  static const double height = 30.0;
  static const double minLabelSpacing = 38.0;
  static const int subBeatDiv = 4;

  @override
  void paint(Canvas canvas, Size size) {
    // 背景：整条（含面板/键盘上方留白）。
    canvas.drawRect(
      Rect.fromLTWH(0, 0, size.width, size.height),
      Paint()..color = YinheTheme.trackBg,
    );
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
    final tpb = ppq.round().clamp(1, 1 << 24);
    final measure = tpb * 4;
    final measurePx = measure * ppu;
    var divisor = 1;
    while (measurePx * divisor < minLabelSpacing && divisor < 64) {
      divisor *= 2;
    }
    final barStep = measure * divisor;

    final showBeat = tpb * ppu >= minLabelSpacing;
    final subTicks = (tpb ~/ subBeatDiv).clamp(1, measure);
    final showSub = subTicks * ppu >= minLabelSpacing;
    final showTick = ppu >= minLabelSpacing;

    final startTick = ((-kbW + scrollX) / ppu).clamp(0.0, double.infinity);
    final endTick = ((size.width - kbW + scrollX) / ppu).clamp(
      0.0,
      double.infinity,
    );
    double xOf(int tick) => tick * ppu - scrollX + kbW;
    // 桌面：文本带中心在标尺高度 0.72 处、左对齐 +2px。
    final yCenter = size.height * 0.72;

    final step = showTick
        ? 1
        : showSub
        ? subTicks
        : showBeat
        ? tpb
        : barStep;
    var tick = (startTick.toInt() ~/ step) * step;
    final endI = endTick.toInt();
    while (tick <= endI + step) {
      final x = xOf(tick);
      if (x >= kbW && x <= size.width + 40) {
        final isMeasure = tick % barStep == 0;
        final isBeat = !isMeasure && tick % tpb == 0;
        String? label;
        Color? color;
        if (isMeasure) {
          label = '${tick ~/ measure + 1}';
          color = YinheTheme.measureLabel;
        } else if (isBeat && showBeat) {
          final bar = tick ~/ measure + 1;
          final beat = (tick % measure) ~/ tpb + 1;
          label = '$bar.$beat';
          color = YinheTheme.textLabel;
        } else if (showSub) {
          final bar = tick ~/ measure + 1;
          final beat = (tick % measure) ~/ tpb + 1;
          if (showTick) {
            label = '$bar.$beat.${(tick % tpb).toString().padLeft(3, '0')}';
            color = YinheTheme.tickLabel;
          } else {
            final sub = (tick % tpb) ~/ subTicks;
            label = '$bar.$beat.$sub';
            color = YinheTheme.textDisabled;
          }
        }
        if (label != null) {
          _label(canvas, label, Offset(x + 2, yCenter), color!);
        }
      }
      tick += step;
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
    required this.trackColors,
    required this.color,
    required this.snap,
    required this.preview,
  }) : super(repaint: Listenable.merge([snap, preview]));

  /// null = 力度模式。
  final AutomationLaneData? lane;

  /// 力度条（每音符/每摘要一条）：(tick, length, velocity01, track)。
  final List<(double, double, double, int)> velocity;

  /// 轨道颜色（力度条按音符所属轨道着色）。
  final List<Color> trackColors;
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
      // 力度条（与桌面 wgpu 一致）：每音符一条空心描边柱，
      // 宽度 = gate × ppu（最小 2px），高度 ∝ (velocity-1)/126；按轨道着色。
      final paint = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.0;
      // 数据按 tick 升序：二分定位第一个进入视口的条，避免每帧全量扫描。
      var lo = 0;
      var hi = velocity.length;
      while (lo < hi) {
        final mid = (lo + hi) >> 1;
        final p = velocity[mid];
        if (xOf(p.$1 + p.$2) < -4) {
          lo = mid + 1;
        } else {
          hi = mid;
        }
      }
      for (var i = lo; i < velocity.length; i++) {
        final (tick, len, vel01, track) = velocity[i];
        final rawLeft = xOf(tick);
        final rawRight = xOf(tick + len);
        if (rawRight < -4) {
          continue;
        }
        if (rawLeft > size.width + 4) {
          break;
        }
        final left = rawLeft.clamp(0.0, size.width);
        var width = rawRight - left;
        if (width < 2.0) {
          width = 2.0;
        }
        final top = yOf(vel01);
        var height = size.height - top;
        if (height < 1.0) {
          height = 1.0;
        }
        paint.color = (track >= 0 && track < trackColors.length)
            ? trackColors[track]
            : color;
        canvas.drawRect(Rect.fromLTWH(left, top, width, height), paint);
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
