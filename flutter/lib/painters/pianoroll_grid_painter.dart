import '../core/snap_fields.dart';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../core/theme.dart';
import 'time_grid.dart';

/// PR 背景：底色、黑键行条纹、时间网格。
///
/// 只画轻量、随视口变化的部分；音符仍由 wgpu 纹理绘制在上层。
/// 网格与选框现在同为 Flutter 浮层，缩放/平移时保持同一帧。
class PianorollGridPainter extends CustomPainter {
  PianorollGridPainter(this.snap, this.gridLines, this.showLabels)
    : super(repaint: Listenable.merge([snap, gridLines, showLabels]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<List<GridLine>> gridLines;

  /// 是否在左侧音名栏显示 C 音名/八度线。
  final ValueListenable<bool> showLabels;

  /// 黑键：每个八度内 C#、D#、F#、G#、A#。
  static bool isBlackKey(int key) {
    const black = {1, 3, 6, 8, 10};
    return black.contains(key % 12);
  }

  @override
  void paint(Canvas canvas, Size size) {
    if (size.width <= 0 || size.height <= 0) {
      return;
    }
    canvas.drawRect(Offset.zero & size, Paint()..color = YinheTheme.bg);
    final s = snap.value;
    if (s.length < 12) {
      return;
    }
    final ppu = s[SnapField.ppu];
    final scrollX = s[SnapField.scrollX];
    final kh = s[SnapField.keyHeight];
    final kbW = s[SnapField.leftPanelW];
    final key0Y = s[SnapField.key0Y];
    final keyStart = s.length > SnapField.keyStart
        ? s[SnapField.keyStart].round()
        : 0;
    final keyCount = s.length > SnapField.keyCount
        ? s[SnapField.keyCount].round()
        : 128;
    if (kh <= 0 || kbW <= 0 || keyCount <= 0) {
      return;
    }

    final stripe = Paint()..color = YinheTheme.stripeBg;
    final white = Paint()..color = YinheTheme.keyWhite;
    final black = Paint()..color = YinheTheme.keyBlack;
    // 与桌面一致：键盘列左半是音名栏，右半才是黑白键。
    final gutterW = kbW * 0.5;
    for (var key = keyStart; key < keyStart + keyCount; key++) {
      final y = key0Y - key * kh;
      if (y + kh < 0 || y > size.height) {
        continue;
      }
      final isBlack = isBlackKey(key);
      if (isBlack) {
        canvas.drawRect(Rect.fromLTWH(0, y, size.width, kh), stripe);
      }
      canvas.drawRect(
        Rect.fromLTWH(gutterW, y, kbW - gutterW, kh),
        isBlack ? black : white,
      );
    }
    canvas.drawRect(
      Rect.fromLTWH(0, 0, gutterW, size.height),
      Paint()..color = YinheTheme.controlBg,
    );
    if (showLabels.value) {
      final font = (kh * 0.5).clamp(7.0, 11.0);
      final firstC = ((keyStart + 11) ~/ 12) * 12;
      final line = Paint()..color = YinheTheme.lineFg;
      for (var key = firstC; key < keyStart + keyCount; key += 12) {
        final y = key0Y - key * kh;
        if (y + kh < 0 || y > size.height) {
          continue;
        }
        canvas.drawRect(Rect.fromLTWH(0, y, gutterW, 1), line);
        final tp = TextPainter(
          text: TextSpan(
            text: 'C${key ~/ 12 - 1}',
            style: TextStyle(color: YinheTheme.textMuted, fontSize: font),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        tp.paint(
          canvas,
          Offset((gutterW - tp.width) / 2, y + (kh - tp.height) / 2),
        );
      }
    }
    // 键盘右缘分隔线。
    canvas.drawRect(
      Rect.fromLTWH(kbW - 1, 0, 1, size.height),
      Paint()..color = YinheTheme.text,
    );

    final g = gridLines.value;
    if (ppu <= 0) {
      return;
    }
    // 网格只画在视口内容区，不能压到钢琴键盘上。
    canvas.save();
    canvas.clipRect(Rect.fromLTWH(kbW, 0, size.width - kbW, size.height));
    paintTimeGrid(
      canvas,
      size: size,
      ppu: ppu,
      scrollX: scrollX,
      leftInset: kbW,
      lines: g,
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => true;
}
