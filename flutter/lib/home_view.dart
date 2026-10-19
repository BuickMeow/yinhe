import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

import 'models.dart';
import 'theme.dart';

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
    required this.onBack,
  });

  final List<RecentItem> recent;
  final VoidCallback onPickMidi;
  final VoidCallback onNewProject;
  final VoidCallback onSaveProjectAs;
  final ValueChanged<RecentItem> onOpenRecent;
  final VoidCallback onOpenSettings;
  final VoidCallback onBack;

  @override
  Widget build(BuildContext context) {
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
              tooltip: '返回工作区',
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
              padding: const EdgeInsets.only(left: 48, right: 40, top: 28),
              child: Row(
                children: [
                  SizedBox(width: 300, child: _actions()),
                  const SizedBox(width: 56),
                  Expanded(child: _recentPanel()),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _actions() {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        FilledButton.icon(
          onPressed: onPickMidi,
          icon: const Icon(Icons.note_add, size: 18),
          label: const Text('打开文件'),
          style: FilledButton.styleFrom(
            backgroundColor: YinheTheme.primaryButtonBg,
            foregroundColor: YinheTheme.text,
            minimumSize: const Size.fromHeight(52),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(6),
            ),
          ),
        ),
        const SizedBox(height: 14),
        _secondary('另存为', Icons.save_as_outlined, onSaveProjectAs),
        const SizedBox(height: 14),
        _secondary('新建工程', Icons.add_box_outlined, onNewProject),
        const SizedBox(height: 14),
        _secondary('设置', Icons.settings, onOpenSettings),
      ],
    );
  }

  Widget _secondary(String label, IconData icon, VoidCallback onTap) {
    return TextButton.icon(
      onPressed: onTap,
      icon: Icon(icon, size: 18),
      label: Text(label, overflow: TextOverflow.ellipsis),
      style: TextButton.styleFrom(
        backgroundColor: YinheTheme.controlBg,
        foregroundColor: YinheTheme.text,
        minimumSize: const Size.fromHeight(52),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
      ),
    );
  }

  Widget _recentPanel() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Text(
          '最近修改的文件',
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
                      '暂无最近文件',
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
