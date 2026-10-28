part of 'studio_shell.dart';

// 视口手势模式（Rust `yinhe_viewport_gesture` 的 mode）。
const int _gesturePan = 0;
const int _gesturePrTime = 1;
const int _gesturePrPitch = 2;
const int _gestureArTime = 3;
const int _gestureArLane = 4;

mixin _ShellInput on _ShellBase {
  // ── 触摸 / 手势 ──

  final Set<int> _activePointers = <int>{};
  final Map<int, Offset> _pointerPositions = <int, Offset>{};

  /// 本手势序列出现过的最大手指数（sticky 到下一次第一根手指按下）。
  /// 用于在多指轻点/长按时压掉单指 Tap/LongPress 的副作用。
  int _maxActivePointers = 0;
  Offset? _gestureStart;
  Offset _lastFocal = Offset.zero;
  double _lastScale = 1.0;

  /// 双指 span（到焦点的平均距离，与 Flutter ScaleGestureRecognizer 同义）。
  ///
  /// 双指平移时手指间距会有亚像素噪声，不能直接把 `ScaleUpdateDetails.scale`
  /// 当成缩放。先在 span 噪声带内只平移，超过 Flutter 同款 scale slop 后才
  /// 进入缩放，避免画面在双指移动时“呼吸”。
  double _pinchStartSpan = 0.0;
  double _pinchLastSpan = 0.0;
  bool _pinchZoomEngaged = false;
  bool _gestureEditing = false;
  bool _gestureWasEditing = false;
  bool _gestureZoomed = false;
  bool _gestureMoved = false;
  bool _pinchModeLocked = false;
  int _gestureMode = _gesturePan;

  bool _isRightMouse(PointerEvent e) =>
      e.kind == PointerDeviceKind.mouse &&
      (e.buttons & kSecondaryMouseButton) != 0;

  /// 双指 span：两指距离的一半，和 Flutter `_ScaleGestureRecognizer._currentSpan`
  /// 对双指的定义一致。
  double _pinchSpan() {
    final points = _pointerPositions.values.toList(growable: false);
    if (points.length < 2) {
      return 0.0;
    }
    return (points[0] - points[1]).distance * 0.5;
  }

  void _resetPinchSpan() {
    _pinchStartSpan = 0.0;
    _pinchLastSpan = 0.0;
    _pinchZoomEngaged = false;
  }

  /// 以当前两指位置重新取缩放基准（指针数量变化、或指针下落不完整时）。
  void _rebasePinchSpan() {
    final span = _pinchSpan();
    _pinchStartSpan = span;
    _pinchLastSpan = span;
    _pinchZoomEngaged = false;
  }

  /// 第一根手指按下时立即 begin，保证铅笔/选框没有 100ms 点击延迟。
  /// 后续的 tap / long press / scale 全部交给 Flutter gesture arena。
  @override
  void _handlePointerDown(PointerDownEvent e) {
    if (_isRightMouse(e)) {
      return; // 右键由原生侧走独立平移通道
    }
    final emptyBefore = _activePointers.isEmpty;
    _activePointers.add(e.pointer);
    _pointerPositions[e.pointer] = e.localPosition;
    if (emptyBefore) {
      _maxActivePointers = 1;
    } else if (_activePointers.length > _maxActivePointers) {
      _maxActivePointers = _activePointers.length;
    }
    if (!emptyBefore) {
      // 第二根手指：取消正在进行的编辑，交给 Flutter 双指手势。
      // 以两指刚落下时的 span 为噪声基准；手势被 arena 接受后再开始累计。
      _rebasePinchSpan();
      rust.editCancel();
      _gestureEditing = false;
      _gestureZoomed = false;
      _pinchModeLocked = false;
      _clearLongPressUi();
      return;
    }

    _gestureStart = e.localPosition;
    _lastFocal = e.localPosition;
    _lastScale = 1.0;
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    _pinchModeLocked = false;
    _gestureMode = _gesturePan;
    rust.stopFling();
    _gestureEditing =
        rust.editBegin(e.localPosition.dx, e.localPosition.dy) == 1;
    _gestureWasEditing = _gestureEditing;
    if (rust.selectionState() == 1) {
      _beginLongPressWatch(e.localPosition);
    } else if (_viewMode == kViewArrangement &&
        _rowAt(e.localPosition) != null) {
      _beginLongPressWatch(e.localPosition, panel: true);
    }
  }

  @override
  void _handlePointerMove(PointerMoveEvent e) {
    if (_pointerPositions.containsKey(e.pointer)) {
      _pointerPositions[e.pointer] = e.localPosition;
    }
  }

  @override
  void _handlePointerUp(PointerUpEvent e) {
    _activePointers.remove(e.pointer);
    _pointerPositions.remove(e.pointer);
    if (_activePointers.isNotEmpty) {
      // 三指变两指：重新取基准；两指变单指：等下一段 ScaleStart 接管平移。
      if (_pointerPositions.length >= 2) {
        _rebasePinchSpan();
      } else {
        _resetPinchSpan();
      }
      return;
    }
    _lastFocal = e.localPosition;
    _pinchModeLocked = false;
    _resetPinchSpan();
    // PointerUp 带精确落点；编辑在最后一根手指离开时立即提交，避免只靠
    // ScaleEnd 的“最后一次 move”导致笔尖/音符结束位置偏几像素。
    if (_gestureEditing) {
      _finishEditGesture(e.localPosition, moved: _gestureMoved || _lpMoved);
    }
  }

  @override
  void _handlePointerCancel(PointerCancelEvent e) {
    _activePointers.remove(e.pointer);
    _pointerPositions.remove(e.pointer);
    if (_activePointers.isNotEmpty) {
      if (_pointerPositions.length >= 2) {
        _rebasePinchSpan();
      } else {
        _resetPinchSpan();
      }
      return;
    }
    rust.editCancel();
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    _pinchModeLocked = false;
    _resetPinchSpan();
    _clearLongPressUi();
  }

  @override
  void _handleTapUp(TapUpDetails d) {
    // 多指轻点由 MultiFingerGestureRecognizer 处理，不能再走单指画布点击
    // （双指 Tap 会在第一根手指抬起时触发一次 onTapUp）。
    if (_maxActivePointers >= 2) {
      _gestureWasEditing = false;
      _clearLongPressUi();
      return;
    }
    final p = d.localPosition;
    if (_gestureEditing) {
      _finishEditGesture(p, moved: _gestureMoved);
    } else {
      _handleCanvasTap(p);
    }
    _gestureWasEditing = false;
    _clearLongPressUi();
  }

  // ── 双指 / 单指平移缩放 ──

  /// 起手锁定缩放轴：内容区只缩放时间轴；PR 键盘列锁音高，AR 面板锁行高。
  int _modeAt(Offset p) {
    final panel = _snap.value.length > 5 ? _snap.value[5] : 0.0;
    if (_viewMode == kViewPianoroll) {
      return p.dx < panel ? _gesturePrPitch : _gesturePrTime;
    }
    return p.dx < panel ? _gestureArLane : _gestureArTime;
  }

  void _lockPinch(Offset p) {
    _pinchModeLocked = true;
    _gestureMode = _modeAt(p);
    rust.editCancel();
    _gestureEditing = false;
    _gestureZoomed = false;
    _clearLongPressUi();
  }

  @override
  void _handleScaleStart(ScaleStartDetails d) {
    rust.stopFling();
    _clearLongPressUi();
    _lastFocal = d.localFocalPoint;
    _lastScale = 1.0;
    _gestureZoomed = false;
    _gestureMoved = false;
    _gestureStart = d.localFocalPoint;
    if (d.pointerCount >= 2) {
      if (_pinchStartSpan <= 0.0) {
        _rebasePinchSpan();
      }
      _lockPinch(d.localFocalPoint);
    } else {
      _resetPinchSpan();
      _pinchModeLocked = false;
      _gestureMode = _gesturePan;
    }
  }

  @override
  void _handleScaleUpdate(ScaleUpdateDetails d) {
    final focal = d.localFocalPoint;
    final pinch = d.pointerCount >= 2 || (d.scale - 1.0).abs() > 0.0001;
    if (pinch) {
      if (_gestureEditing) {
        rust.editCancel();
        _gestureEditing = false;
      }
      if (!_pinchModeLocked) {
        // 从单指平移过渡到双指：以当前焦点为基准重新起手，避免第一帧跳变。
        _pinchModeLocked = true;
        _gestureMode = _modeAt(focal);
        if (_pinchStartSpan <= 0.0) {
          _rebasePinchSpan();
        }
        _lastFocal = focal;
        _lastScale = d.scale;
        return;
      }
      final pan = focal - _lastFocal;
      final span = _pinchSpan();
      double scaleDelta;
      if (span > 0.0 && _pinchStartSpan > 0.0) {
        if (!_pinchZoomEngaged) {
          // 双指平移的 span 噪声带：还没超过 Flutter 同款 scale slop 时只平移。
          if ((span - _pinchStartSpan).abs() < kScaleSlop) {
            scaleDelta = 1.0;
          } else {
            // 第一次确认是缩放：把门限前累积的 span 一次应用，避免起缩跳变。
            scaleDelta = span / _pinchStartSpan;
            _pinchZoomEngaged = true;
            _pinchLastSpan = span;
          }
        } else {
          final last = _pinchLastSpan > 0.0 ? _pinchLastSpan : span;
          scaleDelta = span / last;
          _pinchLastSpan = span;
        }
      } else {
        // 触控板等没有逐指位置的回退路径：沿用累计 scale。
        final last = _lastScale <= 0.0 ? 1.0 : _lastScale;
        scaleDelta = d.scale / last;
        _lastScale = d.scale;
      }
      rust.viewportGesture(
        _gestureMode,
        pan.dx,
        pan.dy,
        scaleDelta,
        focal.dx,
        focal.dy,
      );
      if ((scaleDelta - 1.0).abs() > 0.0001) {
        _gestureZoomed = true;
      }
      _lastFocal = focal;
      _lastScale = d.scale;
      return;
    }

    if (_pointerPositions.length < 2) {
      _resetPinchSpan();
    }
    final pan = focal - _lastFocal;
    if (_gestureEditing) {
      final active = rust.editUpdate(focal.dx, focal.dy) == 1;
      if (!active) {
        // 已选框长按待定被拖动取消：从这次位移开始转为平移。
        _gestureEditing = false;
        rust.viewportGesture(
          _gesturePan,
          pan.dx,
          pan.dy,
          1.0,
          focal.dx,
          focal.dy,
        );
      }
    } else {
      rust.viewportGesture(
        _gesturePan,
        pan.dx,
        pan.dy,
        1.0,
        focal.dx,
        focal.dy,
      );
    }
    if (_gestureStart != null && (focal - _gestureStart!).distance > 3.0) {
      _gestureMoved = true;
    }
    _lastFocal = focal;
    _lastScale = d.scale;
  }

  @override
  void _handleScaleEnd(ScaleEndDetails d) {
    final wasEditing = _gestureEditing || _gestureWasEditing;
    if (_gestureEditing) {
      _finishEditGesture(_lastFocal, moved: _gestureMoved || _lpMoved);
    } else if (!_gestureZoomed && !wasEditing) {
      final v = d.velocity.pixelsPerSecond;
      if (v.distance > 0) {
        rust.startFling(v.dx, v.dy);
      }
    }
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    _pinchModeLocked = false;
    _gestureMode = _gesturePan;
    if (_pointerPositions.length < 2) {
      _resetPinchSpan();
    }
    _clearLongPressUi();
  }

  // ── 长按（Flutter LongPressGestureRecognizer）──

  @override
  void _handleLongPressStart(LongPressStartDetails d) {
    // 多指按住不应触发 AR 拖轨/选区菜单等单指长按行为。
    if (_maxActivePointers >= 2) {
      _clearLongPressUi();
      return;
    }
    final p = d.localPosition;
    if (_viewMode == kViewArrangement) {
      final idx = _rowAt(p);
      if (idx != null && idx < _tracks.value.length) {
        // AR 音轨面板：长按拖动排序。
        _clearLongPressUi();
        _trackDragMoved = false;
        setState(() {
          _dragFrom.value = idx;
          _dragTo.value = idx;
          _dragging = true;
        });
        return;
      }
      if (idx != null && idx >= _tracks.value.length) {
        // 列表末尾是“添加音轨”占位行，由普通 tap 处理。
        _clearLongPressUi();
        return;
      }
    }
    if (!_gestureEditing) {
      _clearLongPressUi();
      return;
    }
    _markLongPressRecognized();
    _lpMoved = false;
    rust.editLongPress();
  }

  void _markLongPressRecognized() {
    _lpStartTime = null;
    _lpProgress.value = 0;
    // 保留 _lpPos：空白处长按松手时用它定位粘贴菜单。
  }

  @override
  void _handleLongPressMove(LongPressMoveUpdateDetails d) {
    final p = d.localPosition;
    if (_dragging) {
      // Master（Conductor）固定在 0 号位，拖动不改目标。
      if (_dragFrom.value <= 0) {
        return;
      }
      final idx = _rowAt(p);
      if (idx == null) {
        return;
      }
      if (idx != _dragFrom.value) {
        _trackDragMoved = true;
      }
      if (idx != _dragTo.value) {
        setState(() => _dragTo.value = idx);
      }
      return;
    }
    if (!_gestureEditing) {
      return;
    }
    final active = rust.editUpdate(p.dx, p.dy) == 1;
    if (!active) {
      _gestureEditing = false;
      return;
    }
    if (_lpPos != null && (p - _lpPos!).distance > 12.0) {
      _lpMoved = true;
    }
    if (_gestureStart != null && (p - _gestureStart!).distance > 3.0) {
      _gestureMoved = true;
    }
  }

  @override
  void _handleLongPressEnd(LongPressEndDetails d) {
    if (_dragging) {
      _finishTrackDrag();
      return;
    }
    if (_gestureEditing) {
      _finishEditGesture(d.localPosition, moved: _lpMoved);
    } else {
      _clearLongPressUi();
    }
  }

  void _finishTrackDrag() {
    final from = _dragFrom.value;
    final to = _dragTo.value;
    setState(() {
      _dragFrom.value = -1;
      _dragTo.value = -1;
      _dragging = false;
    });
    if (from < 0) {
      return;
    }
    if (from == 0) {
      // Master 轨只弹信息菜单，不参与排序。
      _showTrackMenu(0);
      return;
    }
    if (to < 0 || (from == to && !_trackDragMoved)) {
      // 长按未拖动：弹出音轨信息菜单。
      _showTrackMenu(from);
      return;
    }
    if (from == to) {
      // 移动过又放回原位：不排序、不弹菜单。
      return;
    }
    if (rust.moveTrack(from, to) == 1) {
      _refreshDocumentMeta();
    }
  }

  /// 统一的编辑收尾：提交 Rust 编辑，再按长按时的状态弹选区/粘贴菜单。
  void _finishEditGesture(Offset p, {required bool moved}) {
    final state = rust.selectionState();
    LongPressContext? ctx;
    if (state == 3) {
      final ptr = rust.longPressContextJson();
      if (ptr != ffi.nullptr) {
        try {
          final decoded = jsonDecode(ptr.toDartString());
          if (decoded is Map<String, dynamic>) {
            ctx = LongPressContext.fromJson(decoded);
          }
        } catch (_) {
          ctx = null;
        } finally {
          rust.freeString(ptr);
        }
      }
    }
    final pos = _lpPos ?? p;
    rust.editEnd(p.dx, p.dy);
    _gestureEditing = false;
    // 编辑提交后立刻刷新撤销/重做图标，不等下一次 66ms 状态轮询。
    _readDocState();
    _clearLongPressUi();
    if (state == 2 && !moved) {
      _showSelectionMenu();
    } else if (state == 3) {
      _showPasteMenu(pos, ctx);
    }
  }

  /// 按下：开始长按计时（圆环转满后由 Flutter long press 接手）。
  void _beginLongPressWatch(Offset p, {bool panel = false}) {
    _lpPos = p;
    _lpPosNotifier.value = p;
    _lpStartTime = DateTime.now();
    _lpMoved = false;
    _lpPanelRing = panel;
    _lpProgress.value = 0;
    _moveDashed.value = false;
  }

  /// 清理长按 UI 状态（不改 Rust 拖拽状态）。
  void _clearLongPressUi() {
    _lpStartTime = null;
    _lpPos = null;
    _lpPosNotifier.value = null;
    _lpProgress.value = 0;
    _lpMoved = false;
    _lpPanelRing = false;
    _lpPreviewTick = null;
  }

  /// 全量清空 Flutter 手势状态，并让 Rust 取消编辑/惯性。
  @override
  void _resetInteraction() {
    rust.resetInteraction();
    _activePointers.clear();
    _pointerPositions.clear();
    _resetPinchSpan();
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    _pinchModeLocked = false;
    _gestureMode = _gesturePan;
    _clearLongPressUi();
    _clearInsertStatus();
  }

  /// 多指手势：双指轻点撤销。
  @override
  void _undoFromGesture() => _runHistoryGesture(_undo);

  /// 多指手势：三指轻点/右滑重做。
  @override
  void _redoFromGesture() => _runHistoryGesture(_redo);

  /// 手势触发撤销/重做前先丢弃未提交编辑与惯性，避免和 Rust 拖拽状态打架。
  /// 不清 `_activePointers`：动作在最后一根手指抬起后触发，指针集合由 up 自己维护。
  void _runHistoryGesture(void Function() action) {
    if (!mounted) {
      return;
    }
    rust.editCancel();
    rust.stopFling();
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    _pinchModeLocked = false;
    _gestureMode = _gesturePan;
    _resetPinchSpan();
    _clearLongPressUi();
    action();
    _readDocState();
  }

  /// 鼠标滚轮 / 触控板滚动：Ctrl（或 Cmd）+ 滚轮缩放（键盘区缩放音高轴，
  /// 其余缩放时间轴），普通滚轮平移时间轴。
  @override
  void _handlePointerSignal(PointerSignalEvent e) {
    if (e is! PointerScrollEvent) {
      return;
    }
    final ctrl =
        HardwareKeyboard.instance.isControlPressed ||
        HardwareKeyboard.instance.isMetaPressed;
    rust.scrollAt(
      e.localPosition.dx,
      e.localPosition.dy,
      e.scrollDelta.dx,
      e.scrollDelta.dy,
      ctrl ? 1 : 0,
    );
  }

  int? _rowAt(Offset p) {
    final s = _snap.value;
    final panel = s[5];
    final laneH = s[6];
    final scrollY = s[3];
    if (laneH <= 0 || p.dx >= panel) {
      return null;
    }
    final idx = ((p.dy + scrollY) / laneH).floor();
    final rows = _tracks.value;
    // idx == rows.length 是列表末尾的"添加音轨"占位行。
    if (idx < 0 || idx > rows.length) {
      return null;
    }
    return idx;
  }

  bool _inMsZone(Offset p) {
    final s = _snap.value;
    final panel = s[5];
    final laneH = s[6];
    if (laneH < 34) {
      return false;
    }
    final btn = (laneH * 0.44).clamp(18.0, 26.0).toDouble();
    final zoneLeft = panel - 6.0 - btn * 2 - 4.0 - 4.0;
    return p.dx >= zoneLeft;
  }

  /// AR：点击轨道面板。M/S 按钮切换；末行 + 打开添加音轨；其余进入 PR。
  void _handleCanvasTap(Offset p) {
    if (_viewMode != kViewArrangement) {
      return;
    }
    final idx = _rowAt(p);
    if (idx == null) {
      return;
    }
    final rows = _tracks.value;
    if (idx == rows.length) {
      _showAddTrackDialog();
      return;
    }
    if (!rows[idx].master && _inMsZone(p)) {
      _toggleMs(p, idx);
      return;
    }
    // 进入 PR：AR 点的那条轨就是当前编辑轨；「全部轨道」默认关闭。
    _showAllTracks = false;
    _selectEditTrack(idx);
    _setView(kViewPianoroll);
  }

  /// 命中 M/S 按钮：M 在左（警告色），S 在右（危险色）。
  void _toggleMs(Offset p, int idx) {
    final s = _snap.value;
    final laneH = s[6];
    final panelW = s[5];
    final btn = (laneH * 0.44).clamp(18.0, 26.0).toDouble();
    const gap = 4.0;
    final right = panelW - 6.0;
    final mLeft = right - btn * 2 - gap;
    final sLeft = right - btn;
    if (p.dx >= mLeft && p.dx <= mLeft + btn) {
      if (rust.toggleTrackMute(idx) == 1) {
        _refreshDocumentMeta();
      }
    } else if (p.dx >= sLeft && p.dx <= sLeft + btn) {
      if (rust.toggleTrackSolo(idx) == 1) {
        _refreshDocumentMeta();
      }
    }
  }
}
