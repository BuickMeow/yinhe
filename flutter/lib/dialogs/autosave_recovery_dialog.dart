import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n.dart';

/// 启动时发现自动保存残留：让用户明确选择恢复还是丢弃。
///
/// 返回 true = 恢复，false/null = 丢弃。
Future<bool> showAutosaveRecoveryDialog(
  BuildContext context, {
  required String name,
  required int savedAt,
}) async {
  final zh = L10n.isZh;
  final time = savedAt > 0
      ? DateTime.fromMillisecondsSinceEpoch(savedAt * 1000)
            .toString()
            .substring(0, 16)
      : '';
  final ok = await showDialog<bool>(
    context: context,
    barrierDismissible: false,
    builder: (ctx) => AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: Text(
        zh ? '发现自动保存' : 'Autosave found',
        style: TextStyle(color: YinheTheme.text, fontSize: 15),
      ),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            name.isEmpty ? (zh ? '未命名工程' : 'Untitled') : name,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(color: YinheTheme.text, fontSize: 13),
          ),
          if (time.isNotEmpty) ...[
            const SizedBox(height: 6),
            Text(
              time,
              style: TextStyle(color: YinheTheme.textMuted, fontSize: 12),
            ),
          ],
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(ctx).pop(false),
          child: Text(zh ? '丢弃' : 'Discard'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(ctx).pop(true),
          child: Text(zh ? '恢复' : 'Restore'),
        ),
      ],
    ),
  );
  return ok == true;
}
