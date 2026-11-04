import 'package:flutter/material.dart';

import '../core/snap_fields.dart';
import '../core/theme.dart';
import '../painters/ruler_painter.dart';

/// 时间标尺上的浮动圆形播放按钮：跟着播放头走，播放头出视口时隐藏。
///
/// 按钮只占标尺高度，允许圆形的左半边盖在标尺左侧空白区（音轨面板/键盘列）
/// 之上；所以它在 shell 里必须放在 MarkerLayer/标尺占位之后。
class RulerPlayButton extends StatelessWidget {
  const RulerPlayButton({
    super.key,
    required this.snap,
    required this.playhead,
    required this.playing,
    required this.onToggle,
  });

  /// 视口快照：`[5]` = 左侧面板宽。
  final ValueNotifier<List<double>> snap;

  /// 播放头 x（逻辑像素，已含左侧面板偏移）。
  final ValueNotifier<double> playhead;

  final bool playing;
  final VoidCallback onToggle;

  static const double diameter = 28.0;

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: Listenable.merge([snap, playhead]),
      builder: (context, _) {
        return LayoutBuilder(
          builder: (context, constraints) {
            final s = snap.value;
            final x = playhead.value;
            final panelW = s.length > SnapField.leftPanelW
                ? s[SnapField.leftPanelW]
                : 0.0;
            final width = constraints.maxWidth;
            // 与 PlayheadPainter 同一可见条件：播放头离开内容视口就隐藏。
            if (x < panelW || x > width) {
              return const SizedBox.shrink();
            }
            return Stack(
              clipBehavior: Clip.none,
              children: [
                Positioned(
                  left: x - diameter / 2,
                  top: (RulerPainter.height - diameter) / 2,
                  width: diameter,
                  height: diameter,
                  child: Material(
                    color: YinheTheme.accent,
                    elevation: 4,
                    shadowColor: Colors.black.withValues(alpha: 0.35),
                    shape: const CircleBorder(),
                    child: InkWell(
                      customBorder: const CircleBorder(),
                      onTap: onToggle,
                      child: Icon(
                        playing
                            ? Icons.pause_rounded
                            : Icons.play_arrow_rounded,
                        size: 18,
                        color: Colors.white,
                      ),
                    ),
                  ),
                ),
              ],
            );
          },
        );
      },
    );
  }
}
