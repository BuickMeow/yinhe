import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';

/// PR/AR 公用的多指手势：双指轻点撤销、三指轻点重做、三指左/右滑撤销/重做。
///
/// 只接管 2/3 指轻点与 3 指横滑：
/// - 双指轻点在**第一根手指抬起时**抢下 arena，压掉 TapRecognizer 的单指点击；
/// - 三指在第三根落下时立即抢下 arena，避免 Scale/LongPress 先接管；
/// - 双指捏合/平移不会 accept，仍交给画布原有的 Scale 手势。
class MultiFingerGestureRecognizer extends OneSequenceGestureRecognizer {
  MultiFingerGestureRecognizer({
    this.onTwoFingerTap,
    this.onThreeFingerTap,
    this.onSwipeLeft,
    this.onSwipeRight,
  });

  VoidCallback? onTwoFingerTap;
  VoidCallback? onThreeFingerTap;
  VoidCallback? onSwipeLeft;
  VoidCallback? onSwipeRight;

  final Map<int, Offset> _starts = <int, Offset>{};
  final Map<int, Offset> _last = <int, Offset>{};
  Duration? _downAt;
  double _maxMove = 0.0;
  int _maxPointers = 0;
  bool _accepted = false;
  bool _invalid = false;
  bool _fired = false;
  int _swipe = 0; // -1 左，1 右

  @override
  String get debugDescription => 'multi-finger (2/3 tap, 3 swipe)';

  @override
  bool isPointerAllowed(PointerDownEvent event) =>
      event.kind == PointerDeviceKind.touch && super.isPointerAllowed(event);

  @override
  void addAllowedPointer(PointerDownEvent event) {
    super.addAllowedPointer(event);
    _starts[event.pointer] = event.localPosition;
    _last[event.pointer] = event.localPosition;
    _downAt ??= event.timeStamp;
    _maxPointers = math.max(_maxPointers, _starts.length);
    if (!_invalid && !_accepted && _starts.length >= 3) {
      // 三指一落就抢 arena：Tap / LongPress / Scale 全部出局。
      _accepted = true;
      resolve(GestureDisposition.accepted);
    }
  }

  @override
  void handleEvent(PointerEvent event) {
    if (event is PointerMoveEvent) {
      final start = _starts[event.pointer];
      if (start == null) {
        return;
      }
      _last[event.pointer] = event.localPosition;
      _maxMove = math.max(_maxMove, (event.localPosition - start).distance);
      if (_accepted && !_fired && _last.length == 3) {
        final d = _centroid(_last) - _centroid(_starts);
        if (d.dx.abs() > kTouchSlop && d.dx.abs() > d.dy.abs() * 1.5) {
          _swipe = d.dx < 0 ? -1 : 1;
        }
      }
      return;
    }

    if (event is PointerUpEvent) {
      if (!_starts.containsKey(event.pointer)) {
        return;
      }
      _last.remove(event.pointer);
      // 双指轻点：必须在第一根手指 up、arena sweep 之前 accept，
      // 否则 TapRecognizer 会先触发一次普通点击。
      if (!_accepted &&
          !_invalid &&
          _starts.length == 2 &&
          _last.length == 1 &&
          _isTap(event.timeStamp)) {
        _accepted = true;
        resolve(GestureDisposition.accepted);
      }
      if (!_accepted) {
        // 第一根手指抬起时已不是轻点（双指平移/缩放/长按）：本序列作废。
        _invalid = true;
      }
      if (_accepted && !_invalid && !_fired && _last.isEmpty) {
        if (_isTap(event.timeStamp)) {
          _fired = true;
          if (_maxPointers == 2) {
            onTwoFingerTap?.call();
          } else if (_maxPointers == 3) {
            onThreeFingerTap?.call();
          }
        } else if (_maxPointers == 3 && _swipe != 0) {
          _fired = true;
          if (_swipe < 0) {
            onSwipeLeft?.call();
          } else {
            onSwipeRight?.call();
          }
        }
      }
      stopTrackingPointer(event.pointer);
      return;
    }

    if (event is PointerCancelEvent) {
      _invalid = true;
      _last.remove(event.pointer);
      stopTrackingPointer(event.pointer);
    }
  }

  bool _isTap(Duration now) =>
      _maxMove <= kTouchSlop && now - (_downAt ?? now) <= kLongPressTimeout;

  static Offset _centroid(Map<int, Offset> points) {
    if (points.isEmpty) {
      return Offset.zero;
    }
    var sum = Offset.zero;
    for (final p in points.values) {
      sum += p;
    }
    return sum / points.length.toDouble();
  }

  @override
  void rejectGesture(int pointer) {
    // Scale/LongPress 抢先：本序列作废，防止只拿到第三指时误判。
    _invalid = true;
    stopTrackingPointer(pointer);
  }

  @override
  void didStopTrackingLastPointer(int pointer) {
    _starts.clear();
    _last.clear();
    _downAt = null;
    _maxMove = 0.0;
    _maxPointers = 0;
    _accepted = false;
    _invalid = false;
    _fired = false;
    _swipe = 0;
  }
}
