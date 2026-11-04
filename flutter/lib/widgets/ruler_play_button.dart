import 'package:flutter/material.dart';

import '../core/snap_fields.dart';
import '../core/theme.dart';
import '../painters/ruler_painter.dart';

/// 时间标尺上的浮动圆形播放按钮：跟着播放头走，播放头出视口时隐藏。
///
/// 按钮只占标尺高度，允许圆形的左半边盖在标尺左侧空白区（音轨面板/键盘列）
/// 之上；所以它在 shell 里必须放在 MarkerLayer/标尺占位之后。
class RulerPlayButton extends StatefulWidget {
  const RulerPlayButton({
    super.key,
    required this.snap,
    required this.playhead,
    required this.playing,
    required this.onToggle,
    required this.onSeek,
    required this.onSeekEnd,
  });

  /// 视口快照：`[5]` = 左侧面板宽。
  final ValueNotifier<List<double>> snap;

  /// 播放头 x（逻辑像素，已含左侧面板偏移）。
  final ValueNotifier<double> playhead;

  final bool playing;
  final VoidCallback onToggle;

  /// 水平拖动时把按钮中心 x 交给调用方（标尺坐标系）。
  final ValueChanged<double> onSeek;
  final VoidCallback onSeekEnd;

  /// 直径与时间标尺高度一致。
  static const double diameter = RulerPainter.height;

  @override
  State<RulerPlayButton> createState() => _RulerPlayButtonState();
}

class _RulerPlayButtonState extends State<RulerPlayButton> {
  double? _dragX;

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: Listenable.merge([widget.snap, widget.playhead]),
      builder: (context, _) {
        return LayoutBuilder(
          builder: (context, constraints) {
            final s = widget.snap.value;
            final x = widget.playhead.value;
            final panelW = s.length > SnapField.leftPanelW
                ? s[SnapField.leftPanelW]
                : 0.0;
            final width = constraints.maxWidth;
            // 与 PlayheadPainter 同一可见条件：播放头离开内容视口就隐藏。
            if (x < panelW || x > width) {
              return const SizedBox.shrink();
            }
            final center = x - RulerPlayButton.diameter / 2;
            return Stack(
              clipBehavior: Clip.none,
              children: [
                Positioned(
                  left: center,
                  top: 0,
                  width: RulerPlayButton.diameter,
                  height: RulerPlayButton.diameter,
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onHorizontalDragStart: (_) => _dragX = x,
                    onHorizontalDragUpdate: (d) {
                      final next = (_dragX ?? x) + d.delta.dx;
                      _dragX = next;
                      widget.onSeek(next);
                    },
                    onHorizontalDragEnd: (_) {
                      _dragX = null;
                      widget.onSeekEnd();
                    },
                    onHorizontalDragCancel: () {
                      _dragX = null;
                      widget.onSeekEnd();
                    },
                    child: Material(
                      color: YinheTheme.accent,
                      elevation: 4,
                      shadowColor: Colors.black.withValues(alpha: 0.35),
                      shape: const CircleBorder(),
                      child: InkWell(
                        customBorder: const CircleBorder(),
                        onTap: widget.onToggle,
                        child: Icon(
                          widget.playing
                              ? Icons.pause_rounded
                              : Icons.play_arrow_rounded,
                          size: 20,
                          color: Colors.white,
                        ),
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
