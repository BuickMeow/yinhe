import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/pinch_zoom.dart';

void main() {
  test('双指平移不改间距时不缩放', () {
    final z = PinchZoom();
    z.rebase(const [Offset(0, 0), Offset(200, 100)]);
    final (h, v) = z.update(const [Offset(30, 40), Offset(230, 140)]);
    expect(h, 1.0);
    expect(v, 1.0);
  });

  test('间距变化未超过门槛不缩放', () {
    final z = PinchZoom();
    z.rebase(const [Offset(0, 0), Offset(200, 0)]);
    // 半间距 100 -> 110，变化 10 < 16。
    final (h, v) = z.update(const [Offset(0, 0), Offset(220, 0)]);
    expect(h, 1.0);
    expect(v, 1.0);
  });

  test('越过门槛的那一帧只建基准，下一帧开始跟手', () {
    final z = PinchZoom();
    z.rebase(const [Offset(0, 0), Offset(200, 0)]);
    // 半间距 100 -> 120，变化 20 > 16：起手但不缩放。
    var (h, v) = z.update(const [Offset(0, 0), Offset(240, 0)]);
    expect(h, 1.0);
    expect(v, 1.0);
    expect(z.zoomX, isTrue);
    expect(z.zoomY, isFalse);
    // 再拉开 24px（半间距 120 -> 132）：横向 1.1 倍，纵向不动。
    (h, v) = z.update(const [Offset(0, 0), Offset(264, 0)]);
    expect(h, closeTo(1.1, 1e-9));
    expect(v, 1.0);
  });

  test('纯纵向捏合只缩放纵向', () {
    final z = PinchZoom();
    z.rebase(const [Offset(0, 0), Offset(0, 200)]);
    z.update(const [Offset(0, 0), Offset(0, 240)]);
    final (h, v) = z.update(const [Offset(0, 0), Offset(0, 264)]);
    expect(h, 1.0);
    expect(v, closeTo(1.1, 1e-9));
    expect(z.zoomX, isFalse);
    expect(z.zoomY, isTrue);
  });

  test('偏 30° 的捏合只缩放主轴', () {
    final z = PinchZoom();
    // 两指沿 30° 方向：半间距 (100, 57.7)，放大 1.4 倍后 x 变化 40、y 变化 23。
    z.rebase(const [Offset(0, 0), Offset(200, 115.4)]);
    // 第一帧越过 slop：只建立基准。
    z.update(const [Offset(0, 0), Offset(280, 161.6)]);
    final (h, v) = z.update(const [Offset(0, 0), Offset(300, 173.1)]);
    expect(h, greaterThan(1.0));
    expect(v, 1.0);
    expect(z.zoomX, isTrue);
    expect(z.zoomY, isFalse);
  });

  test('接近 45° 的捏合才双轴缩放', () {
    final z = PinchZoom();
    z.rebase(const [Offset(0, 0), Offset(200, 200)]);
    z.update(const [Offset(0, 0), Offset(260, 260)]);
    final (h, v) = z.update(const [Offset(0, 0), Offset(280, 280)]);
    expect(h, greaterThan(1.0));
    expect(v, closeTo(h, 1e-9));
  });

  test('斜向捏合两轴同时缩放', () {
    final z = PinchZoom();
    z.rebase(const [Offset(0, 0), Offset(200, 200)]);
    z.update(const [Offset(0, 0), Offset(240, 240)]);
    final (h, v) = z.update(const [Offset(0, 0), Offset(264, 264)]);
    expect(h, closeTo(1.1, 1e-9));
    expect(v, closeTo(1.1, 1e-9));
  });

  test('rebase 后重新按门槛起手，不会沿用上一轮的缩放状态', () {
    final z = PinchZoom();
    z.rebase(const [Offset(0, 0), Offset(200, 0)]);
    z.update(const [Offset(0, 0), Offset(260, 0)]);
    expect(z.zoomX, isTrue);
    z.rebase(const [Offset(0, 0), Offset(200, 0)]);
    expect(z.zoomX, isFalse);
    final (h, v) = z.update(const [Offset(0, 0), Offset(210, 0)]);
    expect(h, 1.0);
    expect(v, 1.0);
  });

  test('少于两指不产生缩放', () {
    final z = PinchZoom();
    z.rebase(const [Offset(10, 10)]);
    final (h, v) = z.update(const [Offset(20, 20)]);
    expect(h, 1.0);
    expect(v, 1.0);
  });
}
