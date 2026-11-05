import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/pr_track_display.dart';

void main() {
  test('彩色/黑白最多选中一个，重复点击关闭', () {
    // 关闭状态：点彩色 -> 彩色；再点彩色 -> 关闭。
    var s = togglePrTrackDisplay(showAll: false, gray: false, pickGray: false);
    expect(s, (showAll: true, gray: false));
    s = togglePrTrackDisplay(showAll: s.showAll, gray: s.gray, pickGray: false);
    expect(s, (showAll: false, gray: false));

    // 关闭状态：点黑白 -> 黑白；再点黑白 -> 关闭。
    s = togglePrTrackDisplay(showAll: false, gray: false, pickGray: true);
    expect(s, (showAll: true, gray: true));
    s = togglePrTrackDisplay(showAll: s.showAll, gray: s.gray, pickGray: true);
    expect(s, (showAll: false, gray: false));
  });

  test('点另一项直接切换，不会两个同时选中', () {
    final gray = togglePrTrackDisplay(
      showAll: false,
      gray: false,
      pickGray: true,
    );
    final colored = togglePrTrackDisplay(
      showAll: gray.showAll,
      gray: gray.gray,
      pickGray: false,
    );
    expect(colored, (showAll: true, gray: false));

    final backToGray = togglePrTrackDisplay(
      showAll: colored.showAll,
      gray: colored.gray,
      pickGray: true,
    );
    expect(backToGray, (showAll: true, gray: true));
  });
}
