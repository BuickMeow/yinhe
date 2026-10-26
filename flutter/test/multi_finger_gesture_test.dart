import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/multi_finger_gesture.dart';

void main() {
  Future<void> pump(WidgetTester tester, MultiFingerGestureRecognizer r) {
    return tester.pumpWidget(
      MaterialApp(
        home: RawGestureDetector(
          behavior: HitTestBehavior.opaque,
          gestures: <Type, GestureRecognizerFactory>{
            MultiFingerGestureRecognizer:
                GestureRecognizerFactoryWithHandlers<
                  MultiFingerGestureRecognizer
                >(() => r, (_) {}),
          },
          child: const SizedBox.expand(),
        ),
      ),
    );
  }

  testWidgets('双指轻点触发一次撤销', (tester) async {
    var undo = 0;
    final r = MultiFingerGestureRecognizer(onTwoFingerTap: () => undo++);
    await pump(tester, r);
    final a = await tester.startGesture(const Offset(100, 100), pointer: 1);
    final b = await tester.startGesture(const Offset(160, 100), pointer: 2);
    await a.up();
    await b.up();
    await tester.pump();
    expect(undo, 1);
  });

  testWidgets('三指轻点触发一次重做', (tester) async {
    var redo = 0;
    final r = MultiFingerGestureRecognizer(onThreeFingerTap: () => redo++);
    await pump(tester, r);
    final a = await tester.startGesture(const Offset(100, 100), pointer: 1);
    final b = await tester.startGesture(const Offset(150, 100), pointer: 2);
    final c = await tester.startGesture(const Offset(200, 100), pointer: 3);
    await a.up();
    await b.up();
    await c.up();
    await tester.pump();
    expect(redo, 1);
  });

  testWidgets('三指右滑触发重做，左滑触发撤销', (tester) async {
    var undo = 0;
    var redo = 0;
    final r = MultiFingerGestureRecognizer(
      onSwipeLeft: () => undo++,
      onSwipeRight: () => redo++,
    );
    await pump(tester, r);
    final a = await tester.startGesture(const Offset(100, 100), pointer: 1);
    final b = await tester.startGesture(const Offset(150, 100), pointer: 2);
    final c = await tester.startGesture(const Offset(200, 100), pointer: 3);
    for (var i = 0; i < 5; i++) {
      await a.moveBy(const Offset(12, 0));
      await b.moveBy(const Offset(12, 0));
      await c.moveBy(const Offset(12, 0));
    }
    await a.up();
    await b.up();
    await c.up();
    await tester.pump();
    expect(redo, 1, reason: '右滑应重做');
    expect(undo, 0);
  });

  testWidgets('双指捏合不触发撤销/重做', (tester) async {
    var fired = 0;
    final r = MultiFingerGestureRecognizer(
      onTwoFingerTap: () => fired++,
      onThreeFingerTap: () => fired++,
      onSwipeLeft: () => fired++,
      onSwipeRight: () => fired++,
    );
    await pump(tester, r);
    final a = await tester.startGesture(const Offset(100, 100), pointer: 1);
    final b = await tester.startGesture(const Offset(200, 100), pointer: 2);
    for (var i = 0; i < 5; i++) {
      await a.moveBy(const Offset(-10, 0));
      await b.moveBy(const Offset(10, 0));
    }
    await a.up();
    await b.up();
    await tester.pump();
    expect(fired, 0);
  });
}
