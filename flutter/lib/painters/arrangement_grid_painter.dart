import 'package:flutter/material.dart';

import '../core/bar_beat.dart';
import '../core/models.dart';
import '../core/theme.dart';
import 'time_grid.dart';

/// AR 背景：底色、隔行条纹、时间网格（轨道面板/色带由 LabelsPainter 画）。
///
/// 与 PR/AM 一样是 Flutter 浮层，选框与网格同帧；音符仍由 wgpu 纹理绘制。
class ArrangementGridPainter extends CustomPainter {
  ArrangementGridPainter(this.snap, this.grid, this.tracks)
    : super(repaint: Listenable.merge([snap, grid, tracks]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<BarBeatGrid?> grid;
  final ValueNotifier<List<TrackRow>> tracks;

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
    final scrollY = s[3];
    final panelW = s[5];
    final laneH = s[6];
    final count = tracks.value.length;
    if (laneH <= 0 || panelW <= 0 || count <= 0) {
      return;
    }

    // 与 Rust `visible_track_range_static` 一致：多算一行，避免首尾缺条。
    final first = (scrollY / laneH).floor().clamp(0, count - 1);
    final last = (first + (size.height / laneH).ceil() + 1).clamp(0, count);
    final stripe = Paint()..color = YinheTheme.stripeBg;
    for (var idx = first; idx < last; idx++) {
      if (idx.isEven) {
        canvas.drawRect(
          Rect.fromLTWH(0, idx * laneH - scrollY, size.width, laneH),
          stripe,
        );
      }
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
      leftInset: panelW,
      grid: g,
      withSubTick: false,
    );
  }

  @override
  bool shouldRepaint(covariant ArrangementGridPainter oldDelegate) => false;
}
