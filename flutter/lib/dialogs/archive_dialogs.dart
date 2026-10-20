import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';
import 'inputs.dart';

/// 压缩包内的 MIDI 条目。
class ArchiveEntryItem {
  const ArchiveEntryItem({required this.name, required this.size});

  final String name;
  final int size;
}

/// 解析压缩包条目列表 JSON。
List<ArchiveEntryItem> parseArchiveEntries(Map<String, dynamic> json) {
  final list = json['entries'];
  if (list is! List) {
    return const [];
  }
  return [
    for (final it in list)
      if (it is Map<String, dynamic>)
        ArchiveEntryItem(
          name: it['name'] as String? ?? '',
          size: (it['size'] as num?)?.toInt() ?? 0,
        ),
  ];
}

String _fmtSize(int bytes) {
  if (bytes <= 0) {
    return '';
  }
  if (bytes >= 1024 * 1024) {
    return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
  }
  if (bytes >= 1024) {
    return '${(bytes / 1024).toStringAsFixed(0)} KB';
  }
  return '$bytes B';
}

/// 压缩包条目选择对话框；返回选中的下标（取消 = null）。
Future<int?> showArchiveEntriesDialog(
  BuildContext context, {
  required String path,
  required List<ArchiveEntryItem> entries,
}) {
  final l = AppLocalizations.of(context);
  final name = path.split('/').last;
  return showDialog<int>(
    context: context,
    builder: (ctx) => YinheDialog(
      title: name,
      titleTrailing: Text(
        '${entries.length} MIDI',
        style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
      ),
      width: 460,
      scroll: false,
      content: SizedBox(
        height: 360,
        child: ListView.builder(
          itemCount: entries.length,
          itemBuilder: (context, i) {
            final e = entries[i];
            return ListTile(
              dense: true,
              leading: Icon(
                Icons.music_note,
                size: 18,
                color: YinheTheme.textSecondary,
              ),
              title: Text(
                e.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontSize: 14),
              ),
              trailing: Text(
                _fmtSize(e.size),
                style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
              ),
              onTap: () => Navigator.of(ctx).pop(i),
            );
          },
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(ctx).pop(),
          child: Text(l.cancel),
        ),
      ],
    ),
  );
}

/// 压缩包密码输入对话框；返回密码（取消 = null）。
Future<String?> showArchivePasswordDialog(BuildContext context, String path) {
  final l = AppLocalizations.of(context);
  final name = path.split('/').last;
  return showTextInputDialog(
    context,
    title: '${l.archivePasswordRequired}（$name）',
    label: l.password,
    obscure: true,
    confirmText: l.open,
  );
}
