import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import '../core/bar_beat.dart';
import '../core/edit_status.dart';
import '../core/ffi.dart' as rust;
import '../core/midi_names.dart';
import '../core/models.dart';
import '../core/snack.dart';
import '../dialogs/automation_anchor_dialog.dart';
import '../dialogs/batch_edit_dialog.dart';
import '../dialogs/sheet.dart';
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
    required this.grid,
    required this.lanes,
    required this.velocity,
    required this.trackColors,
    required this.activeTrack,
    required this.isConductor,
    required this.status,
    required this.formatTick,
    required this.onCollapsed,
    required this.onReload,
    required this.textureId,
    required this.pencilActive,
    required this.deselectTick,
    required this.onVelocityCommit,
    required this.velocityDrawEnabled,
    required this.lineTool,
    required this.marqueeEnabled,
    required this.onInsertAtTick,
    required this.onResize,
    required this.onResetHeight,
  });

  final double height;
  final ValueNotifier<List<double>> snap;

  /// 拍号分段（与 PR 共用，保证小节/拍网格对齐）。
  final ValueNotifier<BarBeatGrid?> grid;
  final List<AutomationLaneData> lanes;
  final List<(double, double, double, int)> velocity;
  final List<Color> trackColors;
  final int activeTrack;

  /// 当前活动轨是 Conductor：AM 只有 Tempo 一种自动化，没有力度模式。
  final bool isConductor;

  /// 顶部栏临时读数（力度 / 自动化 / 框选）。
  final ValueNotifier<EditStatus?> status;

  /// tick → 顶部栏位置文本（跟随时间码的小节/秒模式）。
  final String Function(double tick) formatTick;
  final ValueChanged<bool> onCollapsed;
  final VoidCallback onReload;

  /// AM wgpu 独立纹理 ID；null = 尚未创建（退回 Flutter 绘制）。
  final int? textureId;

  /// 铅笔工具：只有铅笔显示锚点并允许直接拖锚点。
  final bool pencilActive;

  /// 顶栏取消选区信号：变化时清空锚点选择。
  final int deselectTick;

  /// 提交一笔力度绘制点 `(tick, velocity 1..127)`。
  final void Function(List<(double, double)> points) onVelocityCommit;

  /// 铅笔工具下才允许在力度栏落笔，避免手形/刷子误改力度。
  final bool velocityDrawEnabled;

  /// 直线工具：拖动起止点后一次性写入力度/自动化。
  final bool lineTool;

  /// 仅选框工具允许在 AM 拖出选框；其它工具不误触框选。
  final bool marqueeEnabled;

  /// 长按"插入"：把插入流程交给壳层（复用 PR/AR 的插入菜单）。
  final void Function(double tick) onInsertAtTick;

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

class _AutomationPanelState extends State<AutomationPanel>
    with SingleTickerProviderStateMixin {
  AppLocalizations get l => AppLocalizations.of(context);

  int? _selectedLane; // null = Velocity
  (int, int)? _pendingTarget; // ensureAutomationLane 后等待 lanes 刷新
  final ValueNotifier<List<(double, double, double, int)>> _velocityPreview =
      ValueNotifier(const []);
  bool _velocityStrokeActive = false;
  final List<(double, double)> _velocityPoints = [];
  final Map<(int, int), (double, double, double, int)> _velocityTouched = {};

  /// AM 锚点选择（按 tick）。
  final Set<int> _selectedTicks = {};

  /// 单个锚点点选：可同时改 tick + value；否则批量只改 tick。
  bool _singleSelection = false;

  /// 拖动/框选状态（绘制区局部坐标）。
  double? _dragAnchorTick;
  double? _dragStartSnapTick;
  double? _dragStartValue;
  bool _marqueeActive = false;
  Offset? _marqueeStart;
  Offset? _marqueeCurrent;

  /// 已完成的选框 tick 区间 `[lo, hi)`（与 PR/AR 选框同语义）。
  /// 松手后保留，直到点空白/换轨/换 lane；绘制时按当前 tick→像素换算，
  /// 滚动/缩放后仍贴合网格。
  (double, double)? _marqueeTicks;

  /// 本次框选使用的量化间隔（框选起手时取一次，避免在 build 里调 FFI）。
  double _marqueeInterval = 1;

  /// 长按蓝色圆环（与主画布 200ms→500ms 的节奏一致）。
  late final Ticker _lpTicker;
  DateTime? _lpStart;
  final ValueNotifier<double> _lpProgress = ValueNotifier(0);
  final ValueNotifier<Offset?> _lpPos = ValueNotifier(null);

  /// old tick -> (new tick, new value)，拖动中的预览。
  Map<int, (int, double)> _movePreview = {};

  /// 离网锚点第一次拖动只吸附到网格，不额外移动。
  bool _awaitSnapBreak = false;

  /// 拖动基准：old tick -> 本次拖动开始时的 tick（离网锚点已吸附）。
  /// 不能每帧用 _movePreview 当基准，否则会把上一帧位移重复叠加，越拖越快。
  final Map<int, int> _dragBaseTicks = <int, int>{};

  /// 直线工具拖动状态（绘制区局部坐标）。
  bool _lineActive = false;
  Offset? _lineStartPos;
  Offset? _lineCurrentPos;
  double? _lineStartTick;
  double? _lineStartValue;

  /// AM wgpu 状态去重缓存。
  double _amW = 0;
  double _amH = 0;
  int _amTrack = -1;
  int _amLane = -2;
  bool _amVisible = false;
  bool _amAnchors = false;

  @override
  void initState() {
    super.initState();
    _lpTicker = createTicker(_onLpTick);
  }

  @override
  void didUpdateWidget(covariant AutomationPanel oldWidget) {
    super.didUpdateWidget(oldWidget);
    // 顶栏点空白：清空 AM 锚点选择（音符选区由 Rust 同时清掉）。
    if (oldWidget.deselectTick != widget.deselectTick) {
      _clearSelection();
    }
    if (oldWidget.lineTool != widget.lineTool) {
      _resetLine();
    }
    if (oldWidget.activeTrack != widget.activeTrack) {
      _selectedLane = widget.isConductor ? 0 : null;
      _pendingTarget = null;
      _clearVelocityStroke();
      _clearSelection();
    } else if (widget.isConductor) {
      _selectedLane = 0;
      _pendingTarget = null;
    } else if (_selectedLane != null) {
      if (_selectedLane! >= widget.lanes.length) {
        _selectedLane = null;
        _clearSelection();
      } else if (_canonicalLabel(widget.lanes[_selectedLane!]) == null) {
        // 旧列表里不支持的 CC（如 CC1/CC91）不再出现在 AM 下拉中。
        _selectedLane = null;
        _clearSelection();
      }
    }
    final pending = _pendingTarget;
    if (pending != null) {
      final idx = _findLane(pending.$1, pending.$2);
      if (idx >= 0) {
        _selectedLane = idx;
        _pendingTarget = null;
      }
    }
    _sendAmState();
  }

  /// 同步 AM wgpu 状态（独立纹理）。
  void _sendAmState({bool visible = true}) {
    final w = _amW;
    final h = _amH;
    final track = widget.activeTrack;
    final lane = _laneIdx ?? -1;
    final showAnchors = widget.pencilActive;
    if (_amVisible == visible &&
        _amW == w &&
        _amH == h &&
        _amTrack == track &&
        _amLane == lane &&
        _amAnchors == showAnchors) {
      return;
    }
    _amVisible = visible;
    _amW = w;
    _amH = h;
    _amTrack = track;
    _amLane = lane;
    _amAnchors = showAnchors;
    rust.setAutomationPanel(
      w,
      h,
      visible ? 1 : 0,
      track,
      lane,
      showAnchors ? 1 : 0,
    );
  }

  /// 把拖动预览推给 wgpu ghost 层；空 map 清除 ghost。
  void _syncGhost() {
    rust.pushAutomationGhost([
      for (final e in _movePreview.entries) (e.key, e.value.$1, e.value.$2),
    ]);
  }

  /// 绘制区布局：同步逻辑尺寸给 Rust；物理尺寸由外层 LayoutBuilder 上报。
  void _onPlotLayout(Size size) {
    if (size.width < 1 || size.height < 1) {
      return;
    }
    _amW = size.width;
    _amH = size.height;
    _sendAmState();
  }

  /// 当前选中的 lane 下标（null = 力度）。
  int? get _laneIdx {
    if (widget.isConductor) {
      return widget.lanes.isEmpty ? null : 0;
    }
    return _selectedLane != null && _selectedLane! < widget.lanes.length
        ? _selectedLane
        : null;
  }

  /// 按插入自动化的 target 语义查找 lane（PB/RPN 兼容 MIDI 导入的 Param 映射）。
  int _findLane(int kind, int number) {
    for (var i = 0; i < widget.lanes.length; i++) {
      if (_matchesTarget(widget.lanes[i], kind, number)) {
        return i;
      }
    }
    return -1;
  }

  bool _matchesTarget(AutomationLaneData lane, int kind, int number) {
    if (lane.kind != kind) {
      return false;
    }
    // PB：Param{ChannelInstrument, PITCH_BEND} 也映射为 kind 0。
    if (kind == 1) {
      return true;
    }
    if (lane.number == number) {
      return true;
    }
    // 导入的 RPN 可能是 Param lane（number=-1），按名称兜底匹配。
    if (kind == 3 && lane.number < 0) {
      final name = MidiNames.instance.rpn[number];
      return name != null && name.isNotEmpty && lane.name.contains(name);
    }
    return false;
  }

  @override
  void dispose() {
    rust.setAutomationPanel(0, 0, 0, widget.activeTrack, _laneIdx ?? -1, 0);
    rust.pushAutomationGhost(const []);
    _lpTicker.dispose();
    _lpProgress.dispose();
    _lpPos.dispose();
    _velocityPreview.dispose();
    super.dispose();
  }

  void _beginLpWatch(Offset p) {
    _lpStart = DateTime.now();
    _lpPos.value = p;
    _lpProgress.value = 0;
    if (!_lpTicker.isActive) {
      _lpTicker.start();
    }
  }

  void _onLpTick(Duration _) {
    final start = _lpStart;
    if (start == null) {
      _lpTicker.stop();
      return;
    }
    final ms = DateTime.now().difference(start).inMilliseconds;
    _lpProgress.value = ms <= 200 ? 0 : ((ms - 200) / 300).clamp(0.0, 1.0);
  }

  void _endLpWatch() {
    _lpStart = null;
    _lpPos.value = null;
    _lpProgress.value = 0;
    if (_lpTicker.isActive) {
      _lpTicker.stop();
    }
  }

  void _clearVelocityStroke() {
    _velocityStrokeActive = false;
    _velocityPoints.clear();
    _velocityTouched.clear();
    _velocityPreview.value = const [];
  }

  void _syncSelection() {
    rust.setAutomationSelection(_selectedTicks);
  }

  bool _selectionMenuBusy = false;

  /// 有选区/锚点时：复制、粘贴、修改、插入、删除 + 批量处理。
  Future<void> _showAutomationSelectionMenu({
    required int tick,
    int? modifyTick,
  }) async {
    final canPaste = rust.automationClipboardEmpty() == 0;
    final action = await showActionSheet(
      context,
      actions: [
        SheetAction(
          value: 'copy',
          label: l.copy,
          icon: Icons.content_copy,
          enabled: _selectedTicks.isNotEmpty,
        ),
        SheetAction(
          value: 'paste',
          label: l.paste,
          icon: Icons.content_paste,
          enabled: canPaste,
        ),
        SheetAction(
          value: 'modify',
          label: l.edit,
          icon: Icons.edit_outlined,
          enabled: modifyTick != null,
        ),
        SheetAction(
          value: 'insert',
          label: l.insert,
          icon: Icons.add_circle_outline,
        ),
        SheetAction(
          value: 'delete',
          label: l.delete,
          icon: Icons.delete_outline,
          danger: true,
          enabled: _selectedTicks.isNotEmpty,
        ),
        SheetAction(
          value: 'batch',
          label: l.batchProcess,
          icon: Icons.tune,
          enabled: _selectedTicks.isNotEmpty,
        ),
      ],
    );
    if (!mounted || action == null) {
      return;
    }
    switch (action) {
      case 'copy':
        if (rust.automationCopy() == 1) {
          showSnack(context, l.copied, duration: const Duration(seconds: 1));
        }
      case 'paste':
        _pasteAutomationAt(tick);
      case 'modify':
        if (modifyTick != null) {
          await _editAnchorAt(modifyTick);
        }
      case 'insert':
        widget.onInsertAtTick(tick.toDouble());
      case 'delete':
        await _confirmDeleteSelection();
      case 'batch':
        await _runSelectionAction(24);
    }
  }

  /// 没有按到锚点/选框：只给粘贴、插入。
  Future<void> _showAutomationEmptyMenu(int tick) async {
    final canPaste = rust.automationClipboardEmpty() == 0;
    final action = await showActionSheet(
      context,
      actions: [
        SheetAction(
          value: 'paste',
          label: l.paste,
          icon: Icons.content_paste,
          enabled: canPaste,
        ),
        SheetAction(
          value: 'insert',
          label: l.insert,
          icon: Icons.add_circle_outline,
        ),
      ],
    );
    if (!mounted || action == null) {
      return;
    }
    switch (action) {
      case 'paste':
        _pasteAutomationAt(tick);
      case 'insert':
        widget.onInsertAtTick(tick.toDouble());
    }
  }

  Future<void> _confirmDeleteSelection() async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(l.deleteSelectionConfirm),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: Text(l.cancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(true),
            child: Text(l.delete),
          ),
        ],
      ),
    );
    if (ok == true && mounted) {
      await _runSelectionAction(11);
    }
  }

  void _pasteAutomationAt(int tick) {
    if (rust.automationPaste(tick.toDouble()) == 1) {
      _clearSelection();
      setState(() {});
      widget.onReload();
    }
  }

  /// 起止点按当前量化档吸附成 tick 区间，与 PR/AR 框选完全一致：
  /// 两端都吸附；区间至少一个量化间隔；命中判定为 `[lo, hi)`。
  (double, double) _snapMarquee(Offset a, Offset b) {
    final ta = _snapTickAt(a.dx);
    final tb = _snapTickAt(b.dx);
    var lo = math.min(ta, tb);
    var hi = math.max(ta, tb);
    if (hi <= lo) {
      hi = lo + _marqueeInterval;
    }
    return (lo, hi);
  }

  Rect? _marqueeRectFromTicks((double, double) range) {
    return Rect.fromLTRB(
      _xOfTick(range.$1),
      0,
      _xOfTick(range.$2),
      widget.canvasHeight,
    );
  }

  Future<void> _runSelectionAction(int id) async {
    if (_selectionMenuBusy) {
      return;
    }
    _selectionMenuBusy = true;
    try {
      if (id == 24) {
        final spec = await showBatchEditDialog(context, automation: true);
        if (!mounted || spec == null) {
          return;
        }
        if (rust.automationBatchEdit(spec.value, spec.tick) == 1) {
          setState(() {});
          widget.onReload();
        }
        return;
      }
      if (rust.selectionAction(1, id) == 1) {
        if (id == 11) {
          _clearSelection();
        }
        setState(() {});
        widget.onReload();
      }
    } finally {
      _selectionMenuBusy = false;
    }
  }

  void _clearSelection() {
    _selectedTicks.clear();
    _singleSelection = false;
    _dragAnchorTick = null;
    _movePreview = {};
    _marqueeActive = false;
    _marqueeStart = null;
    _marqueeCurrent = null;
    _marqueeTicks = null;
    _awaitSnapBreak = false;
    _dragBaseTicks.clear();
    _syncGhost();
    _syncSelection();
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

  /// 与 PR 共用量化档位：`tickAtX` 需要画布内容坐标（AM 的 combo 列宽即键盘宽）。
  double _snapTickAt(double localX) =>
      rust.tickAtX(localX + AutomationPainter.comboWidth);

  double _xOfTick(double tick) {
    final s = widget.snap.value;
    return tick * s[1] - s[2] + (s[5] - AutomationPainter.comboWidth);
  }

  double _yOfValue(double value) =>
      widget.canvasHeight * (1.0 - value.clamp(0.0, 1.0));

  double _valueAtY(double y) =>
      (1.0 - (y / widget.canvasHeight)).clamp(0.0, 1.0);

  double _velocityAt(double y) {
    final v = 1.0 + 126.0 * (1.0 - (y / widget.canvasHeight).clamp(0.0, 1.0));
    return v.clamp(1.0, 127.0);
  }

  /// 命中锚点（屏幕欧氏距离 24px 内，触摸目标更大）。
  double? _hitAnchor(AutomationLaneData lane, Offset p) {
    double? best;
    var bestDist = 24.0 * 24.0;
    for (final q in lane.points) {
      final dx = _xOfTick(q.dx) - p.dx;
      final dy = _yOfValue(q.dy) - p.dy;
      final dist = dx * dx + dy * dy;
      if (dist < bestDist) {
        bestDist = dist;
        best = q.dx;
      }
    }
    return best;
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

  void _setVelocityStatus((double, double) p) {
    widget.status.value = EditStatus(
      position: widget.formatTick(p.$1),
      primary: p.$2.round().toString(),
    );
  }

  bool _startVelocityStroke(Offset p) {
    if (!widget.velocityDrawEnabled || _laneIdx != null) {
      return false;
    }
    final point = (_tickAt(p.dx), _velocityAt(p.dy));
    _velocityStrokeActive = true;
    _velocityPoints
      ..clear()
      ..add(point);
    _velocityTouched.clear();
    _hitVelocitySegment(point, point);
    _setVelocityStatus(point);
    return true;
  }

  void _handleVelocityTap(Offset p) {
    if (!widget.velocityDrawEnabled || _laneIdx != null) {
      return;
    }
    final point = (_tickAt(p.dx), _velocityAt(p.dy));
    _setVelocityStatus(point);
    widget.onVelocityCommit([point]);
    widget.status.value = null;
  }

  void _handleVelocityUpdate(Offset p) {
    if (!_velocityStrokeActive) {
      return;
    }
    final point = (_tickAt(p.dx), _velocityAt(p.dy));
    final last = _velocityPoints.last;
    _velocityPoints.add(point);
    _hitVelocitySegment(last, point);
    _setVelocityStatus(point);
  }

  void _finishVelocityStroke() {
    if (!_velocityStrokeActive) {
      return;
    }
    final points = List<(double, double)>.unmodifiable(_velocityPoints);
    _clearVelocityStroke();
    widget.status.value = null;
    if (points.isNotEmpty) {
      widget.onVelocityCommit(points);
    }
  }

  /// 选中集合里若有离网锚点，先把它们吸附到最近网格；第一次拖动不额外位移。
  /// 返回是否进入“等待跨过量化线”的状态。
  bool _snapSelectionToGrid(AutomationLaneData lane) {
    final preview = <int, (int, double)>{};
    _dragBaseTicks.clear();
    for (final p in lane.points) {
      final oldTick = p.dx.round();
      if (!_selectedTicks.contains(oldTick)) {
        continue;
      }
      final snapped = _snapTickAt(_xOfTick(p.dx)).round();
      _dragBaseTicks[oldTick] = snapped;
      if (snapped != oldTick) {
        preview[oldTick] = (snapped, p.dy);
      }
    }
    if (preview.isEmpty) {
      return false;
    }
    _movePreview = preview;
    return true;
  }

  void _resetLine() {
    _lineActive = false;
    _lineStartPos = null;
    _lineCurrentPos = null;
    _lineStartTick = null;
    _lineStartValue = null;
  }

  /// 直线起点：记住吸附 tick 与当前值（力度模式用 velocity01）。
  void _startLine(Offset pos) {
    _lineActive = true;
    _lineStartPos = pos;
    _lineCurrentPos = pos;
    _lineStartTick = _snapTickAt(pos.dx);
    _lineStartValue = _laneIdx == null
        ? _velocityAt(pos.dy) / 127.0
        : _valueAtY(pos.dy);
    _selectedTicks.clear();
    _movePreview = {};
    _syncGhost();
    setState(() {});
  }

  /// 直线松手：按当前工具写入力度/自动化（Rust 侧按 PR 量化补点、一笔 undo）。
  void _finishLine() {
    final startPos = _lineStartPos;
    final endPos = _lineCurrentPos;
    final t0 = _lineStartTick;
    final v0 = _lineStartValue;
    _resetLine();
    setState(() {});
    if (startPos == null || endPos == null || t0 == null || v0 == null) {
      return;
    }
    final t1 = _snapTickAt(endPos.dx);
    final v1 = _laneIdx == null
        ? _velocityAt(endPos.dy) / 127.0
        : _valueAtY(endPos.dy);
    final laneIdx = _laneIdx;
    final ok = laneIdx == null
        ? rust.velocityLine(t0, v0, t1, v1)
        : rust.automationLine(
            widget.lanes[laneIdx].kind,
            widget.lanes[laneIdx].number,
            widget.activeTrack,
            t0,
            v0,
            t1,
            v1,
          );
    if (ok == 1) {
      widget.onReload();
    }
  }

  /// 长按由手势竞技场赢过拖动，按住不动不会进入 [_handlePanStart]。
  ///
  /// 命中锚点（铅笔）或在已有选框内 → 选区菜单；否则 → 插入/粘贴菜单。
  Future<void> _handleLongPressStart(LongPressStartDetails d) async {
    _endLpWatch();
    final laneIdx = _laneIdx;
    if (laneIdx == null || laneIdx >= widget.lanes.length) {
      return;
    }
    final lane = widget.lanes[laneIdx];
    final tick = _snapTickAt(d.localPosition.dx).round();
    final hit = widget.pencilActive ? _hitAnchor(lane, d.localPosition) : null;
    final hitTick = hit?.round();
    final inMarquee =
        _marqueeTicks != null &&
        (_marqueeRectFromTicks(_marqueeTicks!)?.contains(d.localPosition) ??
            false);
    if (hitTick != null) {
      if (!_selectedTicks.contains(hitTick)) {
        _selectedTicks
          ..clear()
          ..add(hitTick);
        _marqueeTicks = null;
      }
      _singleSelection = _selectedTicks.length == 1;
      _movePreview = {};
      _syncGhost();
      _syncSelection();
      setState(() {});
      await _showAutomationSelectionMenu(tick: tick, modifyTick: hitTick);
      return;
    }
    if (inMarquee && _selectedTicks.isNotEmpty) {
      await _showAutomationSelectionMenu(
        tick: tick,
        modifyTick: _selectedTicks.length == 1 ? _selectedTicks.first : null,
      );
      return;
    }
    await _showAutomationEmptyMenu(tick);
  }

  /// 修改单个锚点：tick + value 弹窗。
  Future<void> _editAnchorAt(int tick) async {
    final laneIdx = _laneIdx;
    if (laneIdx == null || laneIdx >= widget.lanes.length) {
      return;
    }
    final lane = widget.lanes[laneIdx];
    final point = lane.points.firstWhere(
      (p) => p.dx.round() == tick,
      orElse: () => Offset(tick.toDouble(), 0),
    );
    final isTempo = lane.kind == 0;
    final max = lane.max > 0 ? lane.max : 1.0;
    final result = await showAutomationAnchorDialog(
      context,
      laneName: _canonicalLabel(lane) ?? _laneLabel(lane),
      tick: tick,
      value: (point.dy.clamp(0.0, 1.0) * max),
      max: max,
      isTempo: isTempo,
    );
    if (result == null || !mounted) {
      return;
    }
    final (newTick, rawValue) = result;
    final normalized = (rawValue / max).clamp(0.0, 1.0);
    final ok = rust.moveAutomationAnchors(widget.activeTrack, laneIdx, [
      (tick, newTick, normalized),
    ]);
    if (ok != 1) {
      return;
    }
    _selectedTicks
      ..clear()
      ..add(newTick);
    _singleSelection = true;
    _syncSelection();
    widget.onReload();
    setState(() {});
  }

  /// 单点锚点编辑 / 框选：
  /// - 按在锚点上：选中（未选中则替换选择），拖动可改 tick+value；
  /// - 按在空白：拉时间框选（全高），多选拖动只改 tick。
  void _handlePanStart(DragStartDetails d) {
    _endLpWatch();
    if (widget.lineTool) {
      _startLine(d.localPosition);
      return;
    }
    if (_laneIdx == null) {
      _startVelocityStroke(d.localPosition);
      return;
    }
    final laneIdx = _laneIdx;
    if (laneIdx == null || laneIdx >= widget.lanes.length) {
      return;
    }
    final s = widget.snap.value;
    if (s[1] <= 0) {
      return;
    }
    final lane = widget.lanes[laneIdx];
    widget.status.value = null;
    final hit = widget.pencilActive ? _hitAnchor(lane, d.localPosition) : null;
    if (hit != null) {
      final hitTick = hit.round();
      if (!_selectedTicks.contains(hitTick)) {
        _selectedTicks
          ..clear()
          ..add(hitTick);
        _marqueeTicks = null;
      }
      _singleSelection = _selectedTicks.length == 1;
      _dragAnchorTick = hit;
      _dragStartSnapTick = _snapTickAt(d.localPosition.dx);
      _dragStartValue = _valueAtY(d.localPosition.dy);
      _movePreview = {};
      _awaitSnapBreak = _snapSelectionToGrid(lane);
      if (_awaitSnapBreak) {
        // 第一次按下就吸附：193 -> 240；在指针跨过下一条量化线前不再移动。
        final moved = _movePreview[_dragAnchorTick!.round()];
        if (moved != null) {
          widget.status.value = EditStatus(
            position: widget.formatTick(moved.$1.toDouble()),
            primary: lane.formatValue(moved.$2),
          );
        }
      }
      _syncGhost();
      _syncSelection();
      setState(() {});
      return;
    }
    // 只有选框工具能拉框；铅笔/手形等其它工具在空白处拖动不改选区。
    if (!widget.marqueeEnabled) {
      return;
    }
    _selectedTicks.clear();
    _singleSelection = false;
    _dragAnchorTick = null;
    _movePreview = {};
    _marqueeTicks = null;
    _marqueeActive = true;
    _marqueeStart = d.localPosition;
    _marqueeCurrent = d.localPosition;
    _marqueeInterval = rust.quantizeTickInterval().toDouble().clamp(
      1.0,
      double.infinity,
    );
    _syncSelection();
    setState(() {});
  }

  void _handlePanUpdate(DragUpdateDetails d) {
    if (_lineActive) {
      setState(() => _lineCurrentPos = d.localPosition);
      return;
    }
    if (_velocityStrokeActive) {
      _handleVelocityUpdate(d.localPosition);
      return;
    }
    if (_marqueeActive) {
      setState(() => _marqueeCurrent = d.localPosition);
      return;
    }
    if (_dragAnchorTick == null ||
        _dragStartSnapTick == null ||
        _dragStartValue == null) {
      return;
    }
    if (_awaitSnapBreak) {
      final now = _snapTickAt(d.localPosition.dx);
      if (now == _dragStartSnapTick) {
        return;
      }
      // 第一次跨过量化线：保持“只吸附”的结果，把位移基准重置到当前位置。
      _dragStartSnapTick = now;
      _dragStartValue = _valueAtY(d.localPosition.dy);
      _awaitSnapBreak = false;
      return;
    }
    final laneIdx = _laneIdx;
    if (laneIdx == null || laneIdx >= widget.lanes.length) {
      return;
    }
    final lane = widget.lanes[laneIdx];
    final dt = _snapTickAt(d.localPosition.dx) - _dragStartSnapTick!;
    final dv = _valueAtY(d.localPosition.dy) - _dragStartValue!;
    // 批量移动保持统一位移：最左锚点碰到 0 就整体停住，读数与实际落点一致。
    final minSel = _selectedTicks.isEmpty ? 0 : _selectedTicks.reduce(math.min);
    final effectiveDt = math.max(dt, -minSel.toDouble());
    final preview = <int, (int, double)>{};
    for (final p in lane.points) {
      final oldTick = p.dx.round();
      if (!_selectedTicks.contains(oldTick)) {
        continue;
      }
      final baseTick = _dragBaseTicks[oldTick] ?? p.dx.round();
      final newTick = math.max(0, (baseTick + effectiveDt).round());
      final newValue = _singleSelection ? (p.dy + dv).clamp(0.0, 1.0) : p.dy;
      preview[oldTick] = (newTick, newValue);
    }
    setState(() => _movePreview = preview);
    _syncGhost();
    if (_singleSelection) {
      final moved = preview[_dragAnchorTick!.round()];
      if (moved != null) {
        widget.status.value = EditStatus(
          position: widget.formatTick(moved.$1.toDouble()),
          primary: lane.formatValue(moved.$2),
        );
      }
    } else {
      final dti = effectiveDt.round();
      widget.status.value = EditStatus(
        hidePosition: true,
        primary: '${dti >= 0 ? '+' : ''}$dti tick',
        secondary: '×${_selectedTicks.length}',
      );
    }
  }

  void _handlePanEnd() {
    if (_lineActive) {
      _finishLine();
      return;
    }
    if (_velocityStrokeActive) {
      _finishVelocityStroke();
      return;
    }
    if (_marqueeActive) {
      _finishMarquee();
      return;
    }
    final laneIdx = _laneIdx;
    final moves = <(int, int, double)>[];
    if (laneIdx != null) {
      for (final p in widget.lanes[laneIdx].points) {
        final oldTick = p.dx.round();
        final moved = _movePreview[oldTick];
        if (moved == null) {
          continue;
        }
        if (moved.$1 == oldTick && (moved.$2 - p.dy).abs() < 1e-6) {
          continue;
        }
        moves.add((oldTick, moved.$1, moved.$2));
      }
    }
    final oldSelected = Set<int>.from(_selectedTicks);
    final wasSingle = _singleSelection;
    _dragAnchorTick = null;
    _dragStartSnapTick = null;
    _dragStartValue = null;
    _movePreview = {};
    _awaitSnapBreak = false;
    _dragBaseTicks.clear();
    _syncGhost();
    widget.status.value = null;
    setState(() {});
    if (moves.isEmpty) {
      return;
    }
    final ok = rust.moveAutomationAnchors(widget.activeTrack, laneIdx!, moves);
    if (ok != 1) {
      _selectedTicks
        ..clear()
        ..addAll(oldSelected);
      _singleSelection = wasSingle;
      _syncSelection();
      setState(() {});
      return;
    }
    _selectedTicks
      ..clear()
      ..addAll(moves.map((m) => m.$2));
    _singleSelection = wasSingle;
    _syncSelection();
    // 立即回读：旧实现延迟 250ms 会让锚点先弹回旧位置再跳回。
    widget.onReload();
    setState(() {});
  }

  void _finishMarquee() {
    final start = _marqueeStart;
    final end = _marqueeCurrent;
    final laneIdx = _laneIdx;
    _marqueeActive = false;
    _marqueeStart = null;
    _marqueeCurrent = null;
    _selectedTicks.clear();
    _singleSelection = false;
    if (start != null &&
        end != null &&
        (end.dx - start.dx).abs() >= 3 &&
        laneIdx != null) {
      // 与 PR/AR 一致：起止点都吸附到当前量化档，选区为 [lo, hi)。
      final range = _snapMarquee(start, end);
      for (final p in widget.lanes[laneIdx].points) {
        if (p.dx >= range.$1 && p.dx < range.$2) {
          _selectedTicks.add(p.dx.round());
        }
      }
      // 松手后选框保留（即使没框到锚点），直到点空白/换轨/换 lane。
      // PR/AR 的选框也不会因为没有命中音符而消失。
      _marqueeTicks = range;
    }
    _syncSelection();
    setState(() {});
  }

  void _handlePanCancel() {
    if (_lineActive) {
      _resetLine();
      setState(() {});
      return;
    }
    if (_velocityStrokeActive) {
      _clearVelocityStroke();
    }
    if (_marqueeActive) {
      _selectedTicks.clear();
      _singleSelection = false;
    }
    _marqueeActive = false;
    _marqueeStart = null;
    _marqueeCurrent = null;
    _dragAnchorTick = null;
    _dragStartSnapTick = null;
    _dragStartValue = null;
    _movePreview = {};
    _awaitSnapBreak = false;
    _dragBaseTicks.clear();
    _syncGhost();
    _syncSelection();
    widget.status.value = null;
    setState(() {});
  }

  Color get _trackColor {
    if (widget.trackColors.isEmpty) {
      return YinheTheme.accent;
    }
    final i = widget.activeTrack.clamp(0, widget.trackColors.length - 1);
    return widget.trackColors[i];
  }

  /// 与「插入自动化」菜单一致的候选 target（kind, number）。
  static const List<(int, int)> _amTargets = <(int, int)>[
    (1, 0), // Pitch Bend
    (2, 7),
    (2, 10),
    (2, 11),
    (2, 64),
    (2, 71),
    (2, 72),
    (2, 73),
    (2, 74),
    (3, 0),
    (3, 1),
    (3, 2),
  ];

  String _laneLabel(AutomationLaneData lane) {
    if (lane.kind == 2 && lane.number >= 0) {
      return MidiNames.instance.ccLabel(lane.number);
    }
    if (lane.kind == 3 && lane.number >= 0) {
      return MidiNames.instance.rpnLabel(lane.number);
    }
    return lane.name;
  }

  /// 规范化 lane 对应的插入菜单文案；非 AM 支持项返回 null。
  String? _canonicalLabel(AutomationLaneData lane) {
    for (final (kind, number) in _amTargets) {
      if (_matchesTarget(lane, kind, number)) {
        return _targetLabel(kind, number);
      }
    }
    return null;
  }

  String _targetLabel(int kind, int number) {
    if (kind == 1) {
      return 'Pitch Bend';
    }
    if (kind == 2) {
      return MidiNames.instance.ccLabel(number);
    }
    return MidiNames.instance.rpnLabel(number);
  }

  @override
  Widget build(BuildContext context) {
    final lane = _laneIdx == null ? null : widget.lanes[_laneIdx!];
    // 只列「插入自动化」支持的标准目标；旧的任意 CC（CC1/CC91…）不再展示。
    // Conductor 没有音符力度，AM 只有 Tempo 一项。
    final entries = <(String, int)>[];
    if (widget.isConductor) {
      final idx = widget.lanes.indexWhere((ln) => ln.kind == 0);
      entries.add(('Tempo', idx >= 0 ? idx + 1 : 0));
    } else {
      entries.add(('Velocity', 0));
    }
    for (final (kind, number) in _amTargets) {
      if (widget.isConductor) {
        break;
      }
      final idx = widget.lanes.indexWhere(
        (ln) => _matchesTarget(ln, kind, number),
      );
      final label = _targetLabel(kind, number);
      if (idx >= 0) {
        entries.add((label, idx + 1));
      } else {
        entries.add((label, -(kind * 100000 + number + 1)));
      }
    }
    Rect? marquee;
    if (_marqueeActive && _marqueeStart != null && _marqueeCurrent != null) {
      final a = _marqueeStart!;
      final b = _marqueeCurrent!;
      marquee = _marqueeRectFromTicks(_snapMarquee(a, b));
    } else if (_marqueeTicks != null) {
      // 松手后的选框常驻；每次按当前 tick→像素重建，滚动/缩放后仍贴合网格。
      marquee = _marqueeRectFromTicks(_marqueeTicks!);
    }
    return Container(
      height: widget.height,
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: YinheTheme.lineFg)),
      ),
      child: Column(
        children: [
          // 顶部拖拽手柄：向上拖变高、向上拖到极限后 PR 只会被压缩不会覆盖。
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onVerticalDragUpdate: (d) => widget.onResize(d.delta.dy),
            onDoubleTap: widget.onResetHeight,
            child: ColoredBox(
              color: YinheTheme.bg,
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
          ),
          Expanded(
            child: Row(
              children: [
                ColoredBox(
                  color: YinheTheme.bg,
                  child: SizedBox(
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
                              for (final (label, value) in entries)
                                PopupMenuItem(value: value, child: Text(label)),
                            ],
                            onSelected: (v) {
                              _clearVelocityStroke();
                              _clearSelection();
                              widget.status.value = null;
                              if (v >= 0) {
                                setState(
                                  () => _selectedLane = v == 0 ? null : v - 1,
                                );
                                _sendAmState();
                                return;
                              }
                              final code = -v - 1;
                              final kind = code ~/ 100000;
                              final number = code % 100000;
                              if (rust.ensureAutomationLane(
                                    widget.activeTrack,
                                    kind,
                                    number,
                                  ) ==
                                  1) {
                                _pendingTarget = (kind, number);
                                widget.onReload();
                              }
                            },
                            child: Center(
                              child: Text(
                                lane == null
                                    ? 'Velocity'
                                    : _canonicalLabel(lane) ?? _laneLabel(lane),
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
                ),
                Expanded(
                  child: Listener(
                    behavior: HitTestBehavior.opaque,
                    onPointerDown: (e) => _beginLpWatch(e.localPosition),
                    onPointerUp: (_) => _endLpWatch(),
                    onPointerCancel: (_) => _endLpWatch(),
                    child: GestureDetector(
                      behavior: HitTestBehavior.opaque,
                      // 锚点单击不选中；只有拖动才进入编辑，避免误触。
                      onTapUp: (d) {
                        _endLpWatch();
                        if (_laneIdx == null) {
                          if (widget.velocityDrawEnabled && !widget.lineTool) {
                            _handleVelocityTap(d.localPosition);
                          }
                          return;
                        }
                        // 选框工具下点空白 = 取消选区/隐藏常驻选框。
                        if (widget.marqueeEnabled) {
                          _clearSelection();
                          setState(() {});
                        }
                      },
                      onLongPressStart: _handleLongPressStart,
                      onPanStart: _handlePanStart,
                      onPanUpdate: _handlePanUpdate,
                      onPanEnd: (_) => _handlePanEnd(),
                      onPanCancel: _handlePanCancel,
                      child: LayoutBuilder(
                        builder: (context, constraints) {
                          _onPlotLayout(constraints.biggest);
                          final gpu = widget.textureId != null;
                          AutomationPainter painter({
                            required bool grid,
                            required bool overlay,
                            required bool base,
                          }) => AutomationPainter(
                            lane: lane,
                            velocity: widget.velocity,
                            trackColors: widget.trackColors,
                            color: _trackColor,
                            snap: widget.snap,
                            grid: widget.grid,
                            selectedTicks: _selectedTicks,
                            movePreview: _movePreview,
                            marquee: marquee,
                            velocityPreview: _velocityPreview,
                            paintBaseData: base,
                            paintGrid: grid,
                            paintOverlay: overlay,
                          );
                          return ClipRect(
                            child: Stack(
                              fit: StackFit.expand,
                              children: [
                                // 背景和网格在 wgpu 纹理下方；纹理透明，数据盖在网格上。
                                const ColoredBox(color: YinheTheme.bg),
                                if (gpu) ...[
                                  CustomPaint(
                                    painter: painter(
                                      grid: true,
                                      overlay: false,
                                      base: false,
                                    ),
                                  ),
                                  Texture(textureId: widget.textureId!),
                                  CustomPaint(
                                    painter: painter(
                                      grid: false,
                                      overlay: true,
                                      base: false,
                                    ),
                                  ),
                                ] else
                                  CustomPaint(
                                    painter: painter(
                                      grid: true,
                                      overlay: true,
                                      base: true,
                                    ),
                                  ),
                                if (_lineActive &&
                                    _lineStartPos != null &&
                                    _lineCurrentPos != null)
                                  IgnorePointer(
                                    child: CustomPaint(
                                      painter: _LinePreviewPainter(
                                        start: _lineStartPos!,
                                        end: _lineCurrentPos!,
                                      ),
                                    ),
                                  ),
                                // 长按蓝色圆环：AM 也要有与主画布一致的按住反馈。
                                IgnorePointer(
                                  child: CustomPaint(
                                    painter: _LongPressRingPainter(
                                      _lpProgress,
                                      _lpPos,
                                    ),
                                  ),
                                ),
                              ],
                            ),
                          );
                        },
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

/// AM 长按进度圆环（与主画布 SelectionLongPressPainter 同节奏/同配色）。
class _LongPressRingPainter extends CustomPainter {
  _LongPressRingPainter(this.progress, this.pos)
    : super(repaint: Listenable.merge([progress, pos]));

  final ValueNotifier<double> progress;
  final ValueNotifier<Offset?> pos;

  @override
  void paint(Canvas canvas, Size size) {
    final p = pos.value;
    final v = progress.value;
    if (p == null || v <= 0 || v >= 1) {
      return;
    }
    const r = 18.0;
    canvas.drawCircle(
      p,
      r,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2
        ..color = YinheTheme.accent.withValues(alpha: 0.2),
    );
    canvas.drawArc(
      Rect.fromCircle(center: p, radius: r),
      -math.pi / 2,
      2 * math.pi * v,
      false,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 3
        ..strokeCap = StrokeCap.round
        ..color = YinheTheme.accent,
    );
  }

  @override
  bool shouldRepaint(covariant _LongPressRingPainter oldDelegate) => false;
}

/// 直线工具的拖动预览：一条线 + 两端圆点。
class _LinePreviewPainter extends CustomPainter {
  const _LinePreviewPainter({required this.start, required this.end});

  final Offset start;
  final Offset end;

  @override
  void paint(Canvas canvas, Size size) {
    final stroke = Paint()
      ..color = YinheTheme.accent
      ..strokeWidth = 1.5
      ..style = PaintingStyle.stroke;
    canvas.drawLine(start, end, stroke);
    final dot = Paint()..color = YinheTheme.accent;
    canvas.drawCircle(start, 4, dot);
    canvas.drawCircle(end, 4, dot);
  }

  @override
  bool shouldRepaint(_LinePreviewPainter old) =>
      old.start != start || old.end != end;
}
