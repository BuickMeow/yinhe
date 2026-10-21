import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';

/// 替换当前工程前的未保存确认结果。
enum UnsavedAction { save, discard, cancel }

/// 询问是否保存当前修改；点外部/返回键按“取消”处理。
Future<UnsavedAction> showUnsavedChangesDialog(BuildContext context) async {
  final l = AppLocalizations.of(context);
  final result = await showDialog<UnsavedAction>(
    context: context,
    builder: (_) => YinheDialog(
      title: l.unsavedChanges,
      width: 420,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(UnsavedAction.cancel),
          child: Text(l.cancel),
        ),
        TextButton(
          onPressed: () => Navigator.of(context).pop(UnsavedAction.discard),
          child: Text(l.dontSave),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(UnsavedAction.save),
          child: Text(l.save),
        ),
      ],
      content: Text(
        l.unsavedChangesPrompt,
        style: TextStyle(color: YinheTheme.textSecondary, fontSize: 14),
      ),
    ),
  );
  return result ?? UnsavedAction.cancel;
}
