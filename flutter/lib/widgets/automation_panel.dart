import 'package:flutter/material.dart';

import '../core/ffi.dart' as rust;
import '../core/models.dart';
import '../painters/painters.dart';
import '../l10n/app_localizations.dart';
import '../core/theme.dart';

/// PR 底部自动化栏（高 81）：combo 列 + 自动化/力度绘制 + 锚点拖动。
class AutomationPanel extends StatefulWidget {
  const AutomationPanel({
    super.key,
    required this.snap,
    required this.lanes,
    required this.velocity,
    required this.trackColors,
    required this.activeTrack,
    required this.onCollapsed,
    required this.onReload,
  });

  final ValueNotifier<List<double>> snap;
  final List<AutomationLaneData> lanes;
  final List<(double, double, double, int)> velocity;
  final List<Color> trackColors;
  final int activeTrack;
  final ValueChanged<bool> onCollapsed;
  final VoidCallback onReload;

  @override
  State<AutomationPanel> createState() => _AutomationPanelState();
}

class _AutomationPanelState extends State<AutomationPanel> {
  AppLocalizations get l => AppLocalizations.of(context);

  int _autoIndex = 0;
  final ValueNotifier<(double, double)?> _autoPreview = ValueNotifier(null);
  bool _draggingAnchor = false;

  @override
  void didUpdateWidget(covariant AutomationPanel oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.activeTrack != widget.activeTrack) {
      _autoIndex = 0;
    } else if (_autoIndex > widget.lanes.length) {
      _autoIndex = 0;
    }
  }

  @override
  void dispose() {
    _autoPreview.dispose();
    super.dispose();
  }

  /// 锚点拖动：按屏幕 x 找最近锚点（离散 lane 的 tick 不变，仅改 value）。
  void _handlePanStart(DragStartDetails d) {
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
    final value = 1.0 - ((d.localPosition.dy - 1) / 80.0);
    setState(() {
      _draggingAnchor = true;
      _autoPreview.value = (tick, value.clamp(0.0, 1.0));
    });
  }

  void _handlePanUpdate(DragUpdateDetails d) {
    final pv = _autoPreview.value;
    if (!_draggingAnchor || pv == null) {
      return;
    }
    final value = 1.0 - ((d.localPosition.dy - 1) / 80.0);
    setState(() => _autoPreview.value = (pv.$1, value.clamp(0.0, 1.0)));
  }

  void _handlePanEnd() {
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
    return Positioned(
      left: 0,
      right: 0,
      bottom: 0,
      child: Container(
        height: 81,
        decoration: BoxDecoration(
          color: YinheTheme.bg,
          border: Border(top: BorderSide(color: YinheTheme.lineFg)),
        ),
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
                      onSelected: (i) => setState(() => _autoIndex = i),
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
                onPanStart: _handlePanStart,
                onPanUpdate: _handlePanUpdate,
                onPanEnd: (_) => _handlePanEnd(),
                child: SizedBox.expand(
                  child: CustomPaint(
                    painter: AutomationPainter(
                      lane: lane,
                      velocity: widget.velocity,
                      trackColors: widget.trackColors,
                      color: _trackColor,
                      snap: widget.snap,
                      preview: _autoPreview,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
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
