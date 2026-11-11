import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/material.dart';

import '../core/ffi.dart' as rust;
import '../core/models.dart';
import '../core/theme.dart';
import '../l10n.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';
import 'inputs.dart';

/// 最近文件弹窗（从侧栏打开）：点击打开，长按重命名 / 从列表移除。
Future<void> showRecentFilesDialog(
  BuildContext context, {
  required List<RecentItem> recent,
  required ValueChanged<RecentItem> onOpen,
  VoidCallback? onChanged,
  ValueChanged<String>? onCurrentRenamed,
}) {
  return showDialog<void>(
    context: context,
    builder: (context) => _RecentFilesDialog(
      recent: recent,
      onOpen: onOpen,
      onChanged: onChanged,
      onCurrentRenamed: onCurrentRenamed,
    ),
  );
}

class _RecentFilesDialog extends StatefulWidget {
  const _RecentFilesDialog({
    required this.recent,
    required this.onOpen,
    this.onChanged,
    this.onCurrentRenamed,
  });

  final List<RecentItem> recent;
  final ValueChanged<RecentItem> onOpen;
  final VoidCallback? onChanged;
  final ValueChanged<String>? onCurrentRenamed;

  @override
  State<_RecentFilesDialog> createState() => _RecentFilesDialogState();
}

class _RecentFilesDialogState extends State<_RecentFilesDialog> {
  late List<RecentItem> _items = List.of(widget.recent);

  void _reload() {
    final ptr = rust.recentJson();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final raw = (ptr.toDartString()).trim();
      final decoded = raw.isEmpty ? const [] : jsonDecode(raw);
      setState(() {
        _items = [
          for (final e in decoded)
            if (e is Map<String, dynamic>) RecentItem.fromJson(e),
        ];
      });
    } catch (_) {
      // 保持旧列表。
    } finally {
      rust.freeString(ptr);
    }
  }

  Future<void> _showMenu(RecentItem item) async {
    final zh = L10n.isZh;
    final action = await showModalBottomSheet<String>(
      context: context,
      backgroundColor: YinheTheme.bg,
      builder: (ctx) => SafeArea(
        top: false,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              dense: true,
              leading: Icon(Icons.edit, size: 20, color: YinheTheme.text),
              title: Text(
                zh ? '重命名' : 'Rename',
                style: TextStyle(color: YinheTheme.text, fontSize: 14),
              ),
              onTap: () => Navigator.of(ctx).pop('rename'),
            ),
            ListTile(
              dense: true,
              leading: Icon(
                Icons.playlist_remove,
                size: 20,
                color: YinheTheme.text,
              ),
              title: Text(
                zh ? '从列表移除' : 'Remove from list',
                style: TextStyle(color: YinheTheme.text, fontSize: 14),
              ),
              onTap: () => Navigator.of(ctx).pop('remove'),
            ),
          ],
        ),
      ),
    );
    if (!mounted || action == null) {
      return;
    }
    if (action == 'remove') {
      final p = item.path.toNativeUtf8();
      try {
        if (rust.recentRemove(p) == 1) {
          _reload();
          widget.onChanged?.call();
        }
      } finally {
        calloc.free(p);
      }
      return;
    }
    final name = await showTextInputDialog(
      context,
      title: zh ? '重命名工程' : 'Rename project',
      label: zh ? '文件名' : 'File name',
      initial: item.name,
      confirmText: zh ? '重命名' : 'Rename',
    );
    if (!mounted || name == null || name.trim().isEmpty) {
      return;
    }
    final current = rust.takeString(rust.currentPath());
    final p = item.path.toNativeUtf8();
    final n = name.trim().toNativeUtf8();
    try {
      if (rust.recentRename(p, n) == 1) {
        var shown = name.trim();
        if (item.name.endsWith('.yin') && !shown.endsWith('.yin')) {
          shown = '$shown.yin';
        }
        if (current == item.path) {
          widget.onCurrentRenamed?.call(shown);
        }
        _reload();
        widget.onChanged?.call();
      }
    } finally {
      calloc.free(p);
      calloc.free(n);
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final height = (MediaQuery.sizeOf(context).height * 0.55).clamp(
      180.0,
      420.0,
    );
    return YinheDialog(
      title: l.recentFiles,
      width: 420,
      scroll: false,
      content: SizedBox(
        height: height,
        child: _items.isEmpty
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
                itemCount: _items.length,
                itemBuilder: (context, i) {
                  final item = _items[i];
                  return GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onLongPress: () => _showMenu(item),
                    child: ListTile(
                      dense: true,
                      contentPadding: const EdgeInsets.symmetric(vertical: 2),
                      leading: Icon(
                        Icons.audio_file,
                        size: 18,
                        color: YinheTheme.textLabel,
                      ),
                      title: Text(
                        item.name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(color: YinheTheme.text, fontSize: 14),
                      ),
                      trailing: Text(
                        _fmtDate(item.time),
                        style: TextStyle(
                          color: YinheTheme.textDisabled,
                          fontSize: 12,
                        ),
                      ),
                      onTap: () {
                        Navigator.of(context).pop();
                        widget.onOpen(item);
                      },
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
    );
  }
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
