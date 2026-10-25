import 'package:flutter/material.dart';

import '../core/ffi.dart' as rust;
import '../core/models.dart';
import '../painters/painters.dart';
import '../l10n/app_localizations.dart';
import '../core/theme.dart';

/// PR 底部自动化栏：combo 列 + 自动化/力度绘制 + 锚点拖动。
///
/// 面板高度由外部控制（[height]），顶部有一条拖拽手柄；展开时 PR 视口会
/// 同步缩短，两者上下排列、互不覆盖。
class AutomationPanel extends StatefulWidget {
  const AutomationPanel({
    super.key,
    required this.height,
    required this.snap,
    required this.lanes,
    required this.velocity,
    required this.trackColors,
    required this.activeTrack,
    required this.onCollapsed,
    required this.onReload,
    required this.onVelocityCommit,
    required this.velocityDrawEnabled,
    required this.onResize,
    required this.onResetHeight,
  });

  final double height;
  final ValueNotifier<List<double>> snap;
  final List<AutomationLaneData> lanes;
  final List<(double, double, double, int)> velocity;
  final List<Color> trackColors;
  final int activeTrack;
  final ValueChanged<bool> onCollapsed;
  final VoidCallback onReload;

  /// 提交一笔力度绘制点 `(tick, velocity 1..127)`。
  final void Function(List<(double, double)> points) onVelocityCommit;

  /// 铅笔工具下才允许在力度栏落笔，避免手形/刷子误改力度。
  final bool velocityDrawEnabled;

  /// 拖动顶部手柄：`dy` 为本次垂直位移（向上为负 = 变高）。
  final ValueChanged<double> onResize;

  /// 双击顶部手柄恢复默认高度。
  final VoidCallback onResetHeight;

  /// 最小高度：保留旧版 81px 绘制区 + 顶部手柄。
  static const double minHeight = 97.0;

  /// 首次展开的默认高度：120px 绘制区，比旧版高一些方便手指绘制。
  static const double defaultHeight = 136.0;

  /// 顶部拖拽手柄占用的高度。
  static const double handleHeight = 16.0;

  /// 绘制区实际可用高度（面板高度扣除拖拽手柄）。
  double get canvasHeight =>
      (height - handleHeight).clamp(1.0, double.infinity);

  @override
  State<AutomationPanel> createState() => _AutomationPanelState();
}

class _AutomationPanelState extends State<AutomationPanel> {
  AppLocalizations get l => AppLocalizations.of(context);

  int _autoIndex = 0;
  final ValueNotifier<(double, double)?> _autoPreview = ValueNotifier(null);
  final ValueNotifier<List<(double, double, double, int)>> _velocityPreview =
      ValueNotifier(const []);
  bool _draggingAnchor = false;
  bool _velocityStrokeActive = false;
  final List<(double, double)> _velocityPoints = [];
  final Map<(int, int), (double, double, double, int)> _velocityTouched = {};

  @override
  void didUpdateWidget(covariant AutomationPanel oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.activeTrack != widget.activeTrack) {
      _autoIndex = 0;
      _clearVelocityStroke();
    } else if (_autoIndex > widget.lanes.length) {
      _autoIndex = 0;
    }
  }

  @override
  void dispose() {
    _autoPreview.dispose();
    _velocityPreview.dispose();
    super.dispose();
  }

  void _clearVelocityStroke() {
    _velocityStrokeActive = false;
    _velocityPoints.clear();
    _velocityTouched.clear();
    _velocityPreview.value = const [];
  }

  double _tickAt(double x) {
    final s = widget.snap.value;
    final ppu = s[1];
    final scrollX = s[2];
    final kbW = s[5];
    if (ppu <= 0) {
      return 0.0;
    }
    final px = x - (kbW - AutomationPainter.comboWidth);
    final tick = (px + scrollX) / ppu;
    return tick < 0.0 ? 0.0 : tick;
  }

  double _velocityAt(double y) {
    final v = 1.0 + 126.0 * (1.0 - (y / widget.canvasHeight).clamp(0.0, 1.0));
    return v.clamp(1.0, 127.0);
  }

  int _velocityLowerBound(double tick) {
    var lo = 0;
    var hi = widget.velocity.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (widget.velocity[mid].$1 < tick) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return lo;
  }

  /// 把一个线段扫过的力度条写入预览。正式修改由 Rust 在松手时按原音符重算。
  void _hitVelocitySegment((double, double) a, (double, double) b) {
    final s = widget.snap.value;
    final ppu = s[1];
    if (ppu <= 0 || widget.activeTrack < 0) {
      return;
    }
    final hit = 2.0 / ppu;
    final lo = (a.$1 < b.$1 ? a.$1 : b.$1) - hit;
    final hi = (a.$1 > b.$1 ? a.$1 : b.$1) + hit;
    final denom = b.$1 - a.$1;
    for (var i = _velocityLowerBound(lo); i < widget.velocity.length; i++) {
      final (tick, len, _, track) = widget.velocity[i];
      if (tick > hi) {
        break;
      }
      if (tick < lo || track != widget.activeTrack) {
        continue;
      }
      final f = denom.abs() < 1e-9 ? 0.0 : (tick - a.$1) / denom;
      final velocity = (a.$2 + (b.$2 - a.$2) * f).clamp(1.0, 127.0);
      _velocityTouched[(tick.round(), track)] = (
        tick,
        len,
        (velocity - 1.0) / 126.0,
        track,
      );
    }
    _velocityPreview.value = _velocityTouched.values.toList(growable: false);
  }

  bool _startVelocityStroke(Offset p) {
    if (!widget.velocityDrawEnabled || _autoIndex != 0) {
      return false;
    }
    final point = (_tickAt(p.dx), _velocityAt(p.dy));
    _velocityStrokeActive = true;
    _velocityPoints
      ..clear()
      ..add(point);
    _velocityTouched.clear();
    _hitVelocitySegment(point, point);
    return true;
  }

  void _handleVelocityTap(Offset p) {
    if (!widget.velocityDrawEnabled || _autoIndex != 0) {
      return;
    }
    final point = (_tickAt(p.dx), _velocityAt(p.dy));
    widget.onVelocityCommit([point]);
  }

  void _handleVelocityUpdate(Offset p) {
    if (!_velocityStrokeActive) {
      return;
    }
    final point = (_tickAt(p.dx), _velocityAt(p.dy));
    final last = _velocityPoints.last;
    _velocityPoints.add(point);
    _hitVelocitySegment(last, point);
  }

  void _finishVelocityStroke() {
    if (!_velocityStrokeActive) {
      return;
    }
    final points = List<(double, double)>.unmodifiable(_velocityPoints);
    _clearVelocityStroke();
    if (points.isNotEmpty) {
      widget.onVelocityCommit(points);
    }
  }

  /// 锚点拖动：按屏幕 x 找最近锚点（离散 lane 的 tick 不变，仅改 value）。
  void _handlePanStart(DragStartDetails d) {
    if (_autoIndex == 0) {
      _startVelocityStroke(d.localPosition);
      return;
    }
    if (_autoIndex < 1 || _autoIndex > widget.lanes.length) {
      return;
    }
    final s = widget.snap.value;
    final ppu = s[1];
    if (ppu <= 0) {
      return;
    }
    final scrollX = s[2];
    final kbW = s[5];
    final localX = d.localPosition.dx;
    double? nearest;
    var bestDist = 10.0;
    for (final p in widget.lanes[_autoIndex - 1].points) {
      final x = p.dx * ppu - scrollX + kbW - AutomationPainter.comboWidth;
      final dist = (x - localX).abs();
      if (dist < bestDist) {
        bestDist = dist;
        nearest = p.dx;
      }
    }
    final tick = nearest;
    if (tick == null) {
      return;
    }
    final value = 1.0 - (d.localPosition.dy / widget.canvasHeight);
    setState(() {
      _draggingAnchor = true;
      _autoPreview.value = (tick, value.clamp(0.0, 1.0));
    });
  }

  void _handlePanUpdate(DragUpdateDetails d) {
    if (_velocityStrokeActive) {
      _handleVelocityUpdate(d.localPosition);
      return;
    }
    final pv = _autoPreview.value;
    if (!_draggingAnchor || pv == null) {
      return;
    }
    final value = 1.0 - (d.localPosition.dy / widget.canvasHeight);
    setState(() => _autoPreview.value = (pv.$1, value.clamp(0.0, 1.0)));
  }

  void _handlePanEnd() {
    if (_velocityStrokeActive) {
      _finishVelocityStroke();
      return;
    }
    final pv = _autoPreview.value;
    final wasDragging = _draggingAnchor;
    setState(() {
      _draggingAnchor = false;
      _autoPreview.value = null;
    });
    if (!wasDragging || pv == null || _autoIndex < 1) {
      return;
    }
    final ok = rust.setAutomationValue(
      widget.activeTrack,
      _autoIndex,
      pv.$1.round(),
      pv.$2,
    );
    if (ok == 1) {
      Future<void>.delayed(const Duration(milliseconds: 250), widget.onReload);
    }
  }

  void _handlePanCancel() {
    if (_velocityStrokeActive) {
      _clearVelocityStroke();
    }
    _draggingAnchor = false;
    _autoPreview.value = null;
  }

  Color get _trackColor {
    if (widget.trackColors.isEmpty) {
      return YinheTheme.accent;
    }
    final i = widget.activeTrack.clamp(0, widget.trackColors.length - 1);
    return widget.trackColors[i];
  }

  @override
  Widget build(BuildContext context) {
    final names = ['Velocity', for (final l in widget.lanes) l.name];
    final index = _autoIndex.clamp(0, names.length - 1).toInt();
    final lane = index >= 1 && index <= widget.lanes.length
        ? widget.lanes[index - 1]
        : null;
    return Container(
      height: widget.height,
      decoration: BoxDecoration(
        color: YinheTheme.bg,
        border: Border(top: BorderSide(color: YinheTheme.lineFg)),
      ),
      child: Column(
        children: [
          // 顶部拖拽手柄：向上拖变高、向上拖到极限后 PR 只会被压缩不会覆盖。
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onVerticalDragUpdate: (d) => widget.onResize(d.delta.dy),
            onDoubleTap: widget.onResetHeight,
            child: SizedBox(
              height: AutomationPanel.handleHeight,
              width: double.infinity,
              child: Center(
                child: Container(
                  width: 36,
                  height: 4,
                  decoration: BoxDecoration(
                    color: YinheTheme.textMuted.withValues(alpha: 0.55),
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              ),
            ),
          ),
          Expanded(
            child: Row(
              children: [
                SizedBox(
                  width: AutomationPainter.comboWidth,
                  child: Column(
                    children: [
                      InkWell(
                        onTap: () => widget.onCollapsed(true),
                        child: SizedBox(
                          height: 22,
                          width: double.infinity,
                          child: Icon(
                            Icons.keyboard_arrow_down,
                            size: 16,
                            color: YinheTheme.textMuted,
                          ),
                        ),
                      ),
                      Expanded(
                        child: PopupMenuButton<int>(
                          tooltip: l.automationType,
                          padding: EdgeInsets.zero,
                          itemBuilder: (_) => [
                            for (var i = 0; i < names.length; i++)
                              PopupMenuItem(value: i, child: Text(names[i])),
                          ],
                          onSelected: (i) {
                            _clearVelocityStroke();
                            setState(() => _autoIndex = i);
                          },
                          child: Center(
                            child: Text(
                              names[index],
                              textAlign: TextAlign.center,
                              maxLines: 2,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                color: YinheTheme.textLabel,
                                fontSize: 10,
                              ),
                            ),
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
                Expanded(
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTapUp: widget.velocityDrawEnabled && _autoIndex == 0
                        ? (d) => _handleVelocityTap(d.localPosition)
                        : null,
                    onPanStart: _handlePanStart,
                    onPanUpdate: _handlePanUpdate,
                    onPanEnd: (_) => _handlePanEnd(),
                    onPanCancel: _handlePanCancel,
                    child: SizedBox.expand(
                      child: CustomPaint(
                        painter: AutomationPainter(
                          lane: lane,
                          velocity: widget.velocity,
                          trackColors: widget.trackColors,
                          color: _trackColor,
                          snap: widget.snap,
                          preview: _autoPreview,
                          velocityPreview: _velocityPreview,
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// 自动化栏收起后的左下角升起按钮。
class AutoExpandButton extends StatelessWidget {
  const AutoExpandButton({super.key, required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Positioned(
      left: 8,
      bottom: 8,
      child: Material(
        color: YinheTheme.controlBg,
        borderRadius: BorderRadius.circular(6),
        elevation: 1,
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.all(5),
            child: Icon(
              Icons.keyboard_arrow_up,
              size: 18,
              color: YinheTheme.textSecondary,
            ),
          ),
        ),
      ),
    );
  }
}
