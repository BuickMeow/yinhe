import 'package:flutter/material.dart';

import '../core/bar_beat.dart';
import '../core/theme.dart';
import 'time_grid.dart';

/// PR 背景：底色、黑键行条纹、八度线、时间网格。
///
/// 只画轻量、随视口变化的部分；音符仍由 wgpu 纹理绘制在上层。
/// 网格与选框现在同为 Flutter 浮层，缩放/平移时保持同一帧。
class PianorollGridPainter extends CustomPainter {
  PianorollGridPainter(this.snap, this.grid)
    : super(repaint: Listenable.merge([snap, grid]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<BarBeatGrid?> grid;

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
    final ppu = s[1];
    final scrollX = s[2];
    final kh = s[4];
    final kbW = s[5];
    final key0Y = s[9];
    if (kh <= 0 || kbW <= 0) {
      return;
    }

    final stripe = Paint()..color = YinheTheme.stripeBg;
    final octave = Paint()..color = YinheTheme.lineFg;
    for (var key = 0; key < 128; key++) {
      final y = key0Y - key * kh;
      if (y + kh < 0 || y > size.height) {
        continue;
      }
      if (isBlackKey(key)) {
        canvas.drawRect(Rect.fromLTWH(0, y, size.width, kh), stripe);
      }
    }
    for (var key = 0; key < 128; key += 12) {
      final y = key0Y - key * kh;
      if (y + kh < 0 || y > size.height) {
        continue;
      }
      canvas.drawRect(Rect.fromLTWH(0, y, size.width, 1), octave);
    }

    final g = grid.value;
    if (g == null || ppu <= 0) {
      return;
    }
    paintTimeGrid(
      canvas,
      size: size,
      ppu: ppu,
      scrollX: scrollX,
      leftInset: kbW,
      grid: g,
    );
  }

  @override
  bool shouldRepaint(covariant PianorollGridPainter oldDelegate) => false;
}
