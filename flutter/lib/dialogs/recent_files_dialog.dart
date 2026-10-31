import 'package:flutter/material.dart';

import '../core/models.dart';
import '../core/theme.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';

/// 最近文件弹窗（从侧栏打开）：列表独立滚动，选中后关闭并打开。
Future<void> showRecentFilesDialog(
  BuildContext context, {
  required List<RecentItem> recent,
  required ValueChanged<RecentItem> onOpen,
}) {
  final l = AppLocalizations.of(context);
  final height = (MediaQuery.sizeOf(context).height * 0.55).clamp(180.0, 420.0);
  return showDialog<void>(
    context: context,
    builder: (context) => YinheDialog(
      title: l.recentFiles,
      width: 420,
      scroll: false,
      content: SizedBox(
        height: height,
        child: recent.isEmpty
            ? Align(
                alignment: Alignment.topLeft,
                child: Text(
                  l.noRecentFiles,
                  style: TextStyle(
                    color: YinheTheme.textDisabled,
                    fontSize: 13,
                  ),
                ),
              )
            : ListView.builder(
                padding: EdgeInsets.zero,
                itemCount: recent.length,
                itemBuilder: (context, i) {
                  final item = recent[i];
                  return InkWell(
                    onTap: () {
                      Navigator.of(context).pop();
                      onOpen(item);
                    },
                    child: Padding(
                      padding: const EdgeInsets.symmetric(vertical: 8),
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
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
      ],
    ),
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
