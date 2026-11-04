import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/snap_fields.dart';
import 'package:yinhe_flutter/widgets/ruler_play_button.dart';

void main() {
  testWidgets('播放头出视口时隐藏，进入视口后跟随并可点击', (tester) async {
    final snap = ValueNotifier<List<double>>(
      List<double>.filled(13, 0.0)..[SnapField.leftPanelW] = 100.0,
    );
    final playhead = ValueNotifier<double>(50.0);
    var taps = 0;

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SizedBox(
            width: 300,
            height: 30,
            child: RulerPlayButton(
              snap: snap,
              playhead: playhead,
              playing: false,
              onToggle: () => taps++,
            ),
          ),
        ),
      ),
    );
    expect(find.byIcon(Icons.play_arrow_rounded), findsNothing);

    playhead.value = 160.0;
    await tester.pump();
    expect(find.byIcon(Icons.play_arrow_rounded), findsOneWidget);

    await tester.tap(find.byIcon(Icons.play_arrow_rounded));
    expect(taps, 1);

    playhead.value = 301.0;
    await tester.pump();
    expect(find.byIcon(Icons.play_arrow_rounded), findsNothing);

    snap.dispose();
    playhead.dispose();
  });
}
