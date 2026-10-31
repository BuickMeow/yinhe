import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n/app_localizations.dart';

/// AR 左侧菜单栏（抽屉）：固定顶部返回栏 + 可滚动选项。
///
/// 顶栏固定是刻意的：后续选项变多时只有选项区滚动，返回按钮始终可见。
class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    required this.onBack,
    required this.onNewProject,
    required this.onOpenFile,
    required this.onSaveAs,
    required this.onExportAudio,
    required this.onExportMidi,
    required this.onRecentFiles,
    required this.onSettings,
  });

  final VoidCallback onBack;
  final VoidCallback onNewProject;
  final VoidCallback onOpenFile;
  final VoidCallback onSaveAs;
  final VoidCallback onExportAudio;
  final VoidCallback onExportMidi;
  final VoidCallback onRecentFiles;
  final VoidCallback onSettings;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return Drawer(
      width: 300,
      backgroundColor: YinheTheme.bg,
      shape: const RoundedRectangleBorder(),
      child: SafeArea(
        child: Column(
          children: [
            _buildHeader(context, l),
            Expanded(
              child: ListView(
                padding: const EdgeInsets.symmetric(vertical: 6),
                children: [
                  _item(Icons.add_box_outlined, l.newProject, onNewProject),
                  _item(Icons.folder_open_outlined, l.openFile, onOpenFile),
                  _item(Icons.save_as_outlined, l.saveAs, onSaveAs),
                  const _SidebarDivider(),
                  _item(
                    Icons.audio_file_outlined,
                    l.exportAudio,
                    onExportAudio,
                  ),
                  _item(
                    Icons.file_download_outlined,
                    l.exportMidi,
                    onExportMidi,
                  ),
                  const _SidebarDivider(),
                  _item(Icons.history, l.recentFiles, onRecentFiles),
                  _item(Icons.settings_outlined, l.settings, onSettings),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildHeader(BuildContext context, AppLocalizations l) {
    return Container(
      height: 48,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      decoration: BoxDecoration(
        color: YinheTheme.controlBg,
        border: Border(bottom: BorderSide(color: YinheTheme.lineFg)),
      ),
      child: Row(
        children: [
          IconButton(
            onPressed: onBack,
            icon: const Icon(Icons.arrow_back),
            color: YinheTheme.text,
            tooltip: l.backToWorkspace,
            visualDensity: VisualDensity.compact,
            padding: EdgeInsets.zero,
            constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
          ),
          const SizedBox(width: 8),
          Text(l.menu, style: TextStyle(color: YinheTheme.text, fontSize: 15)),
        ],
      ),
    );
  }

  Widget _item(IconData icon, String label, VoidCallback onTap) {
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
        child: Row(
          children: [
            Icon(icon, size: 20, color: YinheTheme.textSecondary),
            const SizedBox(width: 14),
            Expanded(
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(color: YinheTheme.text, fontSize: 14),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SidebarDivider extends StatelessWidget {
  const _SidebarDivider();

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Divider(height: 1, color: YinheTheme.lineFg),
    );
  }
}
