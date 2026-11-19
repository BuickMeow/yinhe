import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/l10n/app_localizations.dart';
import 'package:yinhe_flutter/widgets/sidebar.dart';

void main() {
  testWidgets('事件列表可以从侧栏直接打开，不依赖画布长按', (tester) async {
    var opened = 0;
    await tester.pumpWidget(
      MaterialApp(
        locale: const Locale('zh'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          drawer: Sidebar(
            onBack: () {},
            onNewProject: () {},
            onOpenFile: () {},
            onImportMidi: () {},
            onEventList: () => opened++,
            onSaveAs: () {},
            onProjectSettings: () {},
            onExportAudio: () {},
            onExportMidi: () {},
            onExportMidiArchive: () {},
            onCover: () {},
            onLibrary: () {},
            onRecentFiles: () {},
            onSettings: () {},
          ),
          body: const SizedBox.shrink(),
        ),
      ),
    );

    tester.state<ScaffoldState>(find.byType(Scaffold)).openDrawer();
    await tester.pumpAndSettle();

    await tester.tap(find.text('事件列表'));
    await tester.pumpAndSettle();

    expect(opened, 1);
  });
}
