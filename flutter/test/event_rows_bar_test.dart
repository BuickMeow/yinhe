import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/models.dart';
import 'package:yinhe_flutter/core/snap_fields.dart';
import 'package:yinhe_flutter/widgets/event_rows_bar.dart';

void main() {
  testWidgets('事件行显示 METER 行，EVENT 菜单位于独立按钮', (tester) async {
    final snap = ValueNotifier<List<double>>(
      List<double>.filled(SnapField.length, 0)..[SnapField.ppu] = 0.1,
    );
    snap.value[SnapField.leftPanelW] = 60;
    final loop = ValueNotifier<LoopState>(LoopState.off);
    final toggles = <(String, bool)>[];

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              EventRowsBar(
                snap: snap,
                loop: loop,
                data: const EventRowsData(
                  meter: [RulerMarker(tick: 0, text: '4/4')],
                ),
                visibleKinds: const ['meter'],
              ),
              EventMenuButton(
                loop: loop,
                enabledKinds: const {'meter'},
                onToggleRow: (kind, enabled) => toggles.add((kind, enabled)),
              ),
            ],
          ),
        ),
      ),
    );

    expect(find.text('EVENT'), findsOneWidget);
    expect(tester.getSize(find.byType(EventRowsBar)).height, 18);

    await tester.tap(find.text('EVENT'));
    await tester.pumpAndSettle();
    expect(find.text('METER'), findsOneWidget);
    expect(find.text('LOOP'), findsOneWidget);

    await tester.tap(find.text('KEY'));
    await tester.pumpAndSettle();
    expect(toggles, contains(('key', true)));
  });
}
