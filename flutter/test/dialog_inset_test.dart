import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/dialogs/frame.dart';

/// 键盘弹出时对话框要上移到键盘上方，并且用自有曲线平滑运动：
/// 系统 inset 的上报可能是跳变的"台阶"，直接绑上去会一顿一顿地升。
void main() {
  testWidgets('键盘弹出时对话框平滑上移，停在键盘上方', (tester) async {
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
                // 与线上壳一致：窗口不随键盘缩放，弹窗只靠自身上移。
                resizeToAvoidBottomInset: false,
                body: YinheDialog(title: '标题', content: Text('内容')),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    Rect titleRect() => tester.getRect(find.text('标题'));
    final centered = titleRect();

    inset.value = 300;
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    final midway = titleRect();
    expect(midway.top, lessThan(centered.top), reason: '键盘弹出后开始上升');
    expect(
      midway.top,
      greaterThan(centered.top - 150),
      reason: '不是瞬移，而是 150ms 平滑动画',
    );

    await tester.pumpAndSettle();
    final raised = titleRect();
    expect(raised.top, closeTo(centered.top - 150, 1), reason: '上移键盘高度的一半');
    expect(raised.bottom, lessThan(914 - 300), reason: '不能被键盘盖住');
  });
}
