import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/edit_status.dart';
import 'package:yinhe_flutter/l10n/app_localizations.dart';
import 'package:yinhe_flutter/widgets/top_bar.dart';

void main() {
  testWidgets('顶栏标题始终位于屏幕正中央，不受状态数字宽度影响', (tester) async {
    final status = ValueNotifier<EditStatus?>(null);
    final timecode = ValueNotifier<String>('1.0.000');
    final docState = ValueNotifier<int>(0);

    Widget app() => MaterialApp(
      locale: const Locale('zh'),
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(
        body: Center(
          child: SizedBox(
            width: 800,
            height: 42,
            child: TopBar(
              isPr: false,
              playing: false,
              timecodeText: timecode,
              editStatus: status,
              quantizeLabel: '自动',
              ppq: 480,
              mainTool: 0,
              quickTool: 2,
              quickSlotActive: false,
              tracks: const [],
              activeTrack: -1,
              showAllTracks: true,
              songTitle: 'Center Title',
              fileName: 'song.mid',
              docState: docState,
              onHome: () {},
              onBack: () {},
              onTogglePlay: () {},
              onStop: () {},
              onToggleTimecode: () {},
              onQuantize: (_) {},
              onTool: (_) {},
              onQuickTool: (_) {},
              onUndo: () {},
              onRedo: () {},
              onSelectTrack: (_) {},
              onToggleAllTracks: () {},
              onProjectSettings: () {},
              onSave: () {},
              saveDone: false,
              saving: false,
              noteCount: 0,
              bpmText: '120',
              sigText: '4/4',
              onEditBpm: () {},
              onEditSig: () {},
            ),
          ),
        ),
      ),
    );

    await tester.pumpWidget(app());
    final before = tester.getCenter(find.text('Center Title'));
    expect(before.dx, closeTo(400, 0.5), reason: '常规状态标题应在正中央');

    // 模拟 PR/AR 选框移动：左侧数字从短变长、时间码隐藏。
    status.value = const EditStatus(
      hidePosition: true,
      primary: '+10080 tick',
      secondary: '+19 key',
    );
    await tester.pump();
    final after = tester.getCenter(find.text('Center Title'));
    expect(after.dx, closeTo(400, 0.5), reason: '显示相对读数后标题仍应在正中央');
    expect(after.dy, closeTo(before.dy, 0.5));
  });
}
