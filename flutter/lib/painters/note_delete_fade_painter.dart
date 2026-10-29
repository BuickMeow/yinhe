import 'package:flutter/material.dart';

/// 单音符删除动画：音符本体立即从模型删除，这里只画一个短暂残影：
/// 先变成选中色，然后透明度快速消失，不移动。
class NoteDeleteFade {
  NoteDeleteFade(this.rect, this.color, this.start);

  final Rect rect;
  final Color color;
  final DateTime start;

  static const Duration duration = Duration(milliseconds: 180);
}

class NoteDeleteFadePainter extends CustomPainter {
  NoteDeleteFadePainter(this.tick, this.fades) : super(repaint: tick);

  final ValueNotifier<int> tick;
  final List<NoteDeleteFade> fades;

  @override
  void paint(Canvas canvas, Size size) {
    if (fades.isEmpty) {
      return;
    }
    final now = DateTime.now();
    for (final f in fades) {
      final t =
          now.difference(f.start).inMicroseconds /
          NoteDeleteFade.duration.inMicroseconds;
      if (t >= 1) {
        continue;
      }
      // 前 22% 保持选中色，之后迅速淡出。
      const hold = 0.22;
      final a = t < hold ? 1.0 : 1.0 - (t - hold) / (1 - hold);
      canvas.drawRect(f.rect, Paint()..color = f.color.withValues(alpha: a));
    }
  }

  @override
  bool shouldRepaint(covariant NoteDeleteFadePainter oldDelegate) => true;
}
