import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/dialogs/frame.dart';

/// 手机布局下对话框必须吸顶：键盘 inset 变化时位置不能跟着动。
/// 小米输入法的 inset 动画很长，跟随 viewInsets 会让弹窗"慢慢升上去"。
void main() {
  Future<Rect> pumpDialog(WidgetTester tester, double inset) async {
    tester.view.physicalSize = const Size(411, 914);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) => MediaQuery(
            data: MediaQuery.of(context)
                .copyWith(viewInsets: EdgeInsets.only(bottom: inset)),
            child: const Scaffold(
              body: YinheDialog(title: '标题', content: Text('内容')),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return tester.getRect(find.text('标题'));
  }

  testWidgets('手机：键盘弹出前后对话框位置不变且贴近顶部', (tester) async {
    final withoutKeyboard = await pumpDialog(tester, 0);
    expect(withoutKeyboard.top, lessThan(100));

    final withKeyboard = await pumpDialog(tester, 320);
    expect(withKeyboard.top, closeTo(withoutKeyboard.top, 0.1));
    final card = tester.getRect(
      find.byWidgetPredicate(
        (w) => w is Material && w.type == MaterialType.card,
      ),
    );
    expect(card.bottom, lessThan(914 - 320), reason: '对话框整体不能被键盘盖住');
  });
}
