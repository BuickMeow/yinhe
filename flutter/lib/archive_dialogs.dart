import 'package:flutter/material.dart';

import 'theme.dart';

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
  final name = path.split('/').last;
  return showDialog<int>(
    context: context,
    builder: (ctx) => AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: Row(
        children: [
          Expanded(
            child: Text(
              name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(fontSize: 16),
            ),
          ),
          Text(
            '${entries.length} 个 MIDI',
            style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
          ),
        ],
      ),
      content: SizedBox(
        width: 460,
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
          child: const Text('取消'),
        ),
      ],
    ),
  );
}

/// 压缩包密码输入对话框；返回密码（取消 = null）。
Future<String?> showArchivePasswordDialog(BuildContext context, String path) {
  final controller = TextEditingController();
  final name = path.split('/').last;
  return showDialog<String>(
    context: context,
    builder: (ctx) => AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: Text('压缩包需要密码（$name）', style: const TextStyle(fontSize: 16)),
      content: SizedBox(
        width: 360,
        child: Padding(
          padding: const EdgeInsets.only(top: 10),
          child: TextField(
            controller: controller,
            autofocus: true,
            obscureText: true,
            style: const TextStyle(fontSize: 14),
            decoration: const InputDecoration(
              labelText: '密码',
              border: OutlineInputBorder(),
              isDense: true,
            ),
            onSubmitted: (v) => Navigator.of(ctx).pop(v),
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(ctx).pop(),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(ctx).pop(controller.text),
          child: const Text('打开'),
        ),
      ],
    ),
  );
}
