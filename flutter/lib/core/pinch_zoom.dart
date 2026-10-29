import 'dart:ui';

/// 双指分轴缩放的起手判定与逐帧倍率。
///
/// Flutter 的 [ScaleUpdateDetails.horizontalScale] / [verticalScale] 是相对
/// 手势被接受那一刻的间距，且接受时已经吃掉了一道识别 slop。若再按“每帧
/// 比例”过滤噪声（例如 1%），双指平移时微小的间距抖动会逐帧累积成缩放。
///
/// 这里改成和 Android `ScaleGestureDetector` 一致的做法：先看手指间距相对
/// 起手的**绝对变化**，超过 [slop] 才开始缩放；起手后每一帧都原样跟手。
class PinchZoom {
  /// 起手门槛（半间距，逻辑像素）。
  ///
  /// Android 用 `touchSlop * 2`（约 16dp），对应两指总间距变化约 32dp。
  /// 达到门槛的那一帧只建立基准、不缩放，避免突然跳变。
  static const double slop = 16.0;

  /// 副轴要同时缩放时，至少达到主轴位移的比例。
  ///
  /// 斜着捏合时，偏 30° 的副轴位移只有主轴的 ~0.58，仍只缩放主轴；
  /// 接近 45°（副轴 ≥ 主轴的 70%）才认为用户确实想双轴缩放。
  static const double axisDominance = 0.7;

  double _baseX = 0.0;
  double _baseY = 0.0;
  double _lastX = 0.0;
  double _lastY = 0.0;
  bool _zoomX = false;
  bool _zoomY = false;

  /// X 轴（横向）是否已经越过起手门槛。
  bool get zoomX => _zoomX;

  /// Y 轴（纵向）是否已经越过起手门槛。
  bool get zoomY => _zoomY;

  /// 两指（或多指）相对焦点的平均 X/Y 偏差（半间距），
  /// 与 Flutter `_currentHorizontalSpan` / `_currentVerticalSpan` 一致。
  static (double, double) spans(List<Offset> points) {
    if (points.length < 2) {
      return (0.0, 0.0);
    }
    var fx = 0.0;
    var fy = 0.0;
    for (final p in points) {
      fx += p.dx;
      fy += p.dy;
    }
    fx /= points.length;
    fy /= points.length;
    var sx = 0.0;
    var sy = 0.0;
    for (final p in points) {
      sx += (p.dx - fx).abs();
      sy += (p.dy - fy).abs();
    }
    return (sx / points.length, sy / points.length);
  }

  /// 以当前手指位置重新取基准（手势开始、手指数变化时调用）。
  void rebase(List<Offset> points) {
    final (sx, sy) = spans(points);
    _baseX = sx;
    _baseY = sy;
    _lastX = sx;
    _lastY = sy;
    _zoomX = false;
    _zoomY = false;
  }

  /// 返回本帧的 `(横向倍率, 纵向倍率)`；1.0 表示该轴不平移缩放。
  (double, double) update(List<Offset> points) {
    final (sx, sy) = spans(points);
    final adx = (sx - _baseX).abs();
    final ady = (sy - _baseY).abs();
    final xOver = adx > slop;
    final yOver = ady > slop;
    // 两轴都越过 slop 时看主次：副轴不到主轴的 70% 不参与，避免斜一点就双轴缩放。
    final xOnly = xOver && yOver && ady < adx * axisDominance;
    final yOnly = yOver && xOver && adx < ady * axisDominance;
    if (!_zoomX && xOver && !yOnly) {
      _zoomX = true;
      _lastX = sx;
    }
    if (!_zoomY && yOver && !xOnly) {
      _zoomY = true;
      _lastY = sy;
    }
    var h = 1.0;
    var v = 1.0;
    if (_zoomX) {
      if (_lastX >= 1.0 && sx >= 1.0) {
        h = sx / _lastX;
      }
      _lastX = sx;
    }
    if (_zoomY) {
      if (_lastY >= 1.0 && sy >= 1.0) {
        v = sy / _lastY;
      }
      _lastY = sy;
    }
    return (h, v);
  }
}
