import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../core/models.dart';
import '../core/theme.dart';
import '../l10n/app_localizations.dart';

/// 主页（仿桌面）：返回、按钮列、最近修改的文件、銀字水印、版本号、设置。
class HomeView extends StatelessWidget {
  const HomeView({
    super.key,
    required this.recent,
    required this.onPickMidi,
    required this.onNewProject,
    required this.onSaveProjectAs,
    required this.onOpenRecent,
    required this.onOpenSettings,
    required this.onExportAudio,
    required this.onExportMidi,
    required this.onBack,
  });

  final List<RecentItem> recent;
  final VoidCallback onPickMidi;
  final VoidCallback onNewProject;
  final VoidCallback onSaveProjectAs;
  final ValueChanged<RecentItem> onOpenRecent;
  final VoidCallback onOpenSettings;
  final VoidCallback onExportAudio;
  final VoidCallback onExportMidi;
  final VoidCallback onBack;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final compact = MediaQuery.sizeOf(context).width < 600;
    return ColoredBox(
      color: YinheTheme.bg,
      child: Stack(
        children: [
          // 右下角銀字水印（品牌 SVG）。
          Positioned(
            right: -40,
            bottom: -60,
            child: IgnorePointer(
              child: SvgPicture.asset(
                'assets/YINHE.svg',
                width: 420,
                colorFilter: ColorFilter.mode(
                  YinheTheme.textFaint,
                  BlendMode.srcIn,
                ),
              ),
            ),
          ),
          // 右下角版本号。
          const Positioned(
            right: 16,
            bottom: 12,
            child: Text(
              '0.1.0',
              style: TextStyle(color: Color(0xFF9A9AA2), fontSize: 12),
            ),
          ),
          // 左上角返回：与工作区顶部栏的返回/主页按钮同 xy、同尺寸。
          Positioned(
            left: 6,
            top: 1,
            width: 40,
            height: 40,
            child: IconButton(
              tooltip: l.backToWorkspace,
              icon: const Icon(Icons.arrow_back),
              color: YinheTheme.text,
              visualDensity: VisualDensity.compact,
              padding: EdgeInsets.zero,
              constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
              onPressed: onBack,
            ),
          ),
          // 主体：左按钮列 + 右最近文件。
          Positioned.fill(
            child: Padding(
              padding: EdgeInsets.only(
                left: compact ? 24 : 48,
                right: compact ? 24 : 40,
                top: compact ? 48 : 28,
                bottom: compact ? 24 : 0,
              ),
              child: compact
                  ? LayoutBuilder(
                      builder: (context, c) => SingleChildScrollView(
                        child: ConstrainedBox(
                          constraints: BoxConstraints(minHeight: c.maxHeight),
                          child: Column(
                            mainAxisAlignment: MainAxisAlignment.center,
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              _actions(context),
                              const SizedBox(height: 24),
                              SizedBox(
                                height: 320,
                                child: _recentPanel(context),
                              ),
                            ],
                          ),
                        ),
                      ),
                    )
                  : Row(
                      children: [
                        SizedBox(
                          width: 300,
                          child: LayoutBuilder(
                            builder: (context, c) => SingleChildScrollView(
                              child: ConstrainedBox(
                                constraints: BoxConstraints(
                                  minHeight: c.maxHeight,
                                ),
                                child: _actions(context),
                              ),
                            ),
                          ),
                        ),
                        const SizedBox(width: 56),
                        Expanded(child: _recentPanel(context)),
                      ],
                    ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _actions(BuildContext context) {
    final l = AppLocalizations.of(context);
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Row(
          children: [
            Expanded(
              child: _secondary(
                l.newProject,
                Icons.add_box_outlined,
                onNewProject,
              ),
            ),
            const SizedBox(width: 10),
            Expanded(child: _primary(l.openFile, Icons.note_add, onPickMidi)),
          ],
        ),
        const SizedBox(height: 10),
        Row(
          children: [
            Expanded(
              child: _secondary(
                l.saveAs,
                Icons.save_as_outlined,
                onSaveProjectAs,
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: _secondary(
                l.exportAudio,
                Icons.audio_file_outlined,
                onExportAudio,
              ),
            ),
          ],
        ),
        const SizedBox(height: 10),
        Row(
          children: [
            Expanded(
              child: _secondary(
                l.exportMidi,
                Icons.file_download_outlined,
                onExportMidi,
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: _secondary(l.settings, Icons.settings, onOpenSettings),
            ),
          ],
        ),
      ],
    );
  }

  Widget _primary(String label, IconData icon, VoidCallback onTap) {
    return FilledButton.icon(
      onPressed: onTap,
      icon: Icon(icon, size: 16),
      label: Text(label, overflow: TextOverflow.ellipsis),
      style: FilledButton.styleFrom(
        backgroundColor: YinheTheme.primaryButtonBg,
        foregroundColor: YinheTheme.text,
        minimumSize: const Size.fromHeight(44),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
      ),
    );
  }

  Widget _secondary(String label, IconData icon, VoidCallback onTap) {
    return TextButton.icon(
      onPressed: onTap,
      icon: Icon(icon, size: 16),
      label: Text(label, overflow: TextOverflow.ellipsis),
      style: TextButton.styleFrom(
        backgroundColor: YinheTheme.controlBg,
        foregroundColor: YinheTheme.text,
        minimumSize: const Size.fromHeight(44),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
      ),
    );
  }

  Widget _recentPanel(BuildContext context) {
    final l = AppLocalizations.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Text(
          l.recentFiles,
          style: TextStyle(color: YinheTheme.textLabel, fontSize: 14),
        ),
        const SizedBox(height: 8),
        Expanded(
          child: recent.isEmpty
              ? Align(
                  alignment: Alignment.topLeft,
                  child: Padding(
                    padding: const EdgeInsets.only(top: 8),
                    child: Text(
                      l.noRecentFiles,
                      style: TextStyle(
                        color: YinheTheme.textDisabled,
                        fontSize: 13,
                      ),
                    ),
                  ),
                )
              : ListView.builder(
                  padding: EdgeInsets.zero,
                  itemCount: recent.length,
                  itemBuilder: (context, i) {
                    final item = recent[i];
                    return InkWell(
                      onTap: () => onOpenRecent(item),
                      child: Padding(
                        padding: const EdgeInsets.symmetric(vertical: 7),
                        child: Row(
                          children: [
                            Icon(
                              Icons.audio_file,
                              size: 18,
                              color: YinheTheme.textLabel,
                            ),
                            const SizedBox(width: 8),
                            Expanded(
                              child: Text(
                                item.name,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: TextStyle(
                                  color: YinheTheme.text,
                                  fontSize: 14,
                                ),
                              ),
                            ),
                            Text(
                              _fmtDate(item.time),
                              style: TextStyle(
                                color: YinheTheme.textDisabled,
                                fontSize: 12,
                              ),
                            ),
                          ],
                        ),
                      ),
                    );
                  },
                ),
        ),
      ],
    );
  }

  String _fmtDate(int epochSeconds) {
    if (epochSeconds <= 0) {
      return '';
    }
    final dt = DateTime.fromMillisecondsSinceEpoch(epochSeconds * 1000);
    String two(int v) => v.toString().padLeft(2, '0');
    return '${dt.year}-${two(dt.month)}-${two(dt.day)} '
        '${two(dt.hour)}:${two(dt.minute)}';
  }
}
