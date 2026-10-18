import 'package:flutter/material.dart';

import 'models.dart';
import 'theme.dart';

/// 文本浮层：AR 轨道面板文字/编号/M-S、拖动排序指示，PR 键盘 C 音名。
class LabelsPainter extends CustomPainter {
  LabelsPainter({
    required this.snap,
    required this.tracks,
    required this.dragFrom,
    required this.dragTo,
    required this.arAutomation,
  }) : super(repaint: Listenable.merge([snap, tracks, dragFrom, dragTo]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<List<TrackRow>> tracks;
  final ValueNotifier<int> dragFrom;
  final ValueNotifier<int> dragTo;

  /// AR 折叠态自动化竖线（与桌面 collect_folded_previews 同语义）。
  final List<ArAutomationLane> arAutomation;

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

  /// AR 折叠态自动化竖线：整层 alpha 0.35（与桌面一致），重叠不加深。
  void _paintArAutomation(
    Canvas canvas,
    Size size,
    List<double> s,
    List<TrackRow> rows,
  ) {
    if (arAutomation.isEmpty) {
      return;
    }
    final ppu = s[1];
    final scrollX = s[2];
    final scrollY = s[3];
    final laneH = s[6];
    final panelW = s[5];
    if (ppu <= 0 || laneH <= 0 || panelW < 0) {
      return;
    }
    // 整层 0.35 合成：同轨多 lane / 重叠竖线不叠加变深（桌面用 Max 混合）。
    canvas.saveLayer(
      Rect.fromLTWH(panelW, 0, size.width - panelW, size.height),
      Paint()..color = Colors.white.withValues(alpha: 0.35),
    );
    final startTick = scrollX / ppu;
    for (final lane in arAutomation) {
      if (lane.track >= rows.length) {
        continue;
      }
      final rowY = lane.track * laneH - scrollY;
      if (rowY + laneH < 0 || rowY > size.height) {
        continue;
      }
      final pts = lane.points;
      if (pts.isEmpty) {
        continue;
      }
      final paint = Paint()
        ..color = rows[lane.track].color
        ..strokeWidth = 1;
      final baseY = rowY + (1 - lane.baseline) * laneH;
      // 二分定位第一个可见事件。
      var lo = 0;
      var hi = pts.length;
      while (lo < hi) {
        final mid = (lo + hi) >> 1;
        if (pts[mid].dx < startTick) {
          lo = mid + 1;
        } else {
          hi = mid;
        }
      }
      for (var i = lo; i < pts.length; i++) {
        final p = pts[i];
        final x = p.dx * ppu - scrollX + panelW;
        if (x < panelW) {
          continue;
        }
        if (x > size.width) {
          break;
        }
        final y = rowY + (1 - p.dy) * laneH;
        canvas.drawLine(Offset(x, baseY), Offset(x, y), paint);
      }
    }
    canvas.restore();
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
    _paintArAutomation(canvas, size, s, rows);
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
    // 色带（sRGB 直绘保持饱和，与桌面 egui badge 一致；wgpu decor 不再画）。
    for (var idx = first; idx < last; idx++) {
      final y = idx * laneH - scrollY;
      canvas.drawRect(
        Rect.fromLTWH(0, y, 6, laneH),
        Paint()..color = rows[idx].color,
      );
    }
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
  bool shouldRepaint(covariant LabelsPainter oldDelegate) =>
      oldDelegate.arAutomation != arAutomation;
}

/// PR 自动化栏曲线（Flutter 自绘：x=tick 变换，y=归一化值 0..1）。
