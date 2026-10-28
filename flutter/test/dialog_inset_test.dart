import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/dialogs/frame.dart';

/// 键盘弹出时对话框上移到固定位置（输入框中心约在屏幕上 20%），
/// 不再依赖键盘高度；位移过程是 150ms 平滑动画。
void main() {
  testWidgets('输入框中心落在屏幕上 20%，且与键盘高度无关', (tester) async {
    tester.view.physicalSize = const Size(411, 914);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    final inset = ValueNotifier<double>(0);
    addTearDown(inset.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) => ValueListenableBuilder<double>(
            valueListenable: inset,
            builder: (context, value, _) => MediaQuery(
              data: MediaQuery.of(context)
                  .copyWith(viewInsets: EdgeInsets.only(bottom: value)),
              child: const Scaffold(
                // 与线上壳一致：窗口不随键盘缩放，弹窗只靠自身移动。
                resizeToAvoidBottomInset: false,
                body: YinheDialog(
                  title: '标题',
                  content: TextField(
                    decoration: InputDecoration(labelText: '歌曲标题'),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    Offset fieldCenter() => tester.getCenter(find.byType(TextField));

    final closedCenter = fieldCenter();

    inset.value = 300;
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    final midway = fieldCenter();
    await tester.pumpAndSettle();
    final openCenter = fieldCenter();
    expect(midway.dy, lessThan(closedCenter.dy), reason: '键盘弹出后开始上移');
    expect(midway.dy, greaterThan(openCenter.dy), reason: '不是瞬移，而是 150ms 平滑动画');
    expect(
      openCenter.dy / 914,
      inInclusiveRange(0.15, 0.25),
      reason: '输入框中心约在屏幕上 20%',
    );

    // 换一个键盘高度：位置必须完全一致（没有用键盘高度计算）。
    inset.value = 200;
    await tester.pumpAndSettle();
    expect(fieldCenter().dy, closeTo(openCenter.dy, 0.1));
  });
}
