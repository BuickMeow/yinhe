import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/edit_status.dart';
import 'package:yinhe_flutter/l10n/app_localizations.dart';
import 'package:yinhe_flutter/widgets/top_bar.dart';

Widget _app({
  required ValueNotifier<EditStatus?> status,
  required ValueNotifier<String> timecode,
  required ValueNotifier<int> docState,
  double width = 800,
  double height = 42,
  bool compact = false,
}) {
  return MaterialApp(
    locale: const Locale('zh'),
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: Scaffold(
      body: Center(
        child: SizedBox(
          width: width,
          height: height,
          child: TopBar(
            height: height,
            compact: compact,
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
            onMenu: () {},
            onBack: () {},
            onTogglePlay: () {},
            onStop: () {},
            onToggleTimecode: () {},
            onQuantize: (_) {},
            automationOpen: false,
            onToggleAutomation: () {},
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
            onDeselect: () {},
          ),
        ),
      ),
    ),
  );
}

void main() {
  testWidgets('顶栏标题始终位于屏幕正中央，不受状态数字宽度影响', (tester) async {
    final status = ValueNotifier<EditStatus?>(null);
    final timecode = ValueNotifier<String>('1.0.000');
    final docState = ValueNotifier<int>(0);

    await tester.pumpWidget(
      _app(status: status, timecode: timecode, docState: docState),
    );
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

  testWidgets('竖屏双行顶栏不溢出，左右两组按钮都在', (tester) async {
    final status = ValueNotifier<EditStatus?>(null);
    final timecode = ValueNotifier<String>('1.0.000');
    final docState = ValueNotifier<int>(4);

    await tester.pumpWidget(
      _app(
        status: status,
        timecode: timecode,
        docState: docState,
        width: 400,
        height: 84,
        compact: true,
      ),
    );

    expect(tester.takeException(), isNull, reason: '竖屏顶栏不应 RenderFlex 溢出');
    expect(find.byIcon(Icons.motion_photos_on), findsOneWidget);
    expect(find.byIcon(Icons.undo), findsOneWidget);
    expect(find.byIcon(Icons.save_outlined), findsOneWidget);
    final bar = tester.getRect(find.byType(TopBar));
    final title = tester.getCenter(find.textContaining('Center Title'));
    expect(title.dx - bar.left, closeTo(bar.width / 2, 1), reason: '竖屏第二行标题居中');
    final menu = tester.getCenter(find.byIcon(Icons.menu));
    expect(menu.dx - bar.left, lessThan(30), reason: '菜单按钮保持在左上角');
    final quantizeY = tester.getCenter(find.byIcon(Icons.motion_photos_on)).dy;
    expect(menu.dy, lessThan(quantizeY), reason: '主运输在第一行，量化在第二行');
    final undoX = tester.getCenter(find.byIcon(Icons.undo)).dx;
    final redoX = tester.getCenter(find.byIcon(Icons.redo)).dx;
    final saveX = tester.getCenter(find.byIcon(Icons.save_outlined)).dx;
    expect(undoX, lessThan(redoX));
    expect(redoX, lessThan(saveX), reason: '右侧顺序应为 撤销/重做/保存');
  });
}
