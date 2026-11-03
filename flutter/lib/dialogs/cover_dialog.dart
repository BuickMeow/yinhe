import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n.dart';
import 'frame.dart';

enum CoverAction { pick, clear }

/// 工程封面操作：选择图片 / 清除 / 取消。
Future<CoverAction?> showCoverDialog(
  BuildContext context, {
  required bool hasCover,
  required int sizeBytes,
}) {
  final zh = L10n.isZh;
  return showDialog<CoverAction>(
    context: context,
    builder: (context) {
      return YinheDialog(
        title: zh ? '工程封面' : 'Project cover',
        width: 420,
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: Text(zh ? '取消' : 'Cancel'),
          ),
          if (hasCover)
            TextButton(
              onPressed: () => Navigator.of(context).pop(CoverAction.clear),
              child: Text(
                zh ? '清除' : 'Clear',
                style: const TextStyle(color: Colors.redAccent),
              ),
            ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(CoverAction.pick),
            child: Text(zh ? '选择图片' : 'Choose image'),
          ),
        ],
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              hasCover
                  ? (zh
                        ? '当前封面：${(sizeBytes / 1024).ceil()} KB'
                        : 'Current cover: ${(sizeBytes / 1024).ceil()} KB')
                  : (zh ? '当前未设置封面。' : 'No cover set.'),
              style: const TextStyle(fontSize: 13),
            ),
            const SizedBox(height: 8),
            Text(
              zh ? '封面会嵌入 .yin，并在导出音频时作为专辑封面写入。' : 'The cover is embedded in .yin and written as album art when exporting audio.',
              style: TextStyle(color: YinheTheme.textSecondary, fontSize: 12),
            ),
          ],
        ),
      );
    },
  );
}
