import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';

/// 通用耗时任务进度对话框；成功返回 true。
///
/// Flutter 没有 Android 的 `ProgressDialog`，惯用实现就是 `showDialog` +
/// 不可点外部关闭 + 进度条；后台线程执行任务，Dart 定时轮询 JSON 状态。
Future<bool?> showTaskProgressDialog(
  BuildContext context, {
  required String Function() poll,
  VoidCallback? onCancel,
  bool determinate = true,
  String? title,
  String? failureTitle,
}) {
  return showDialog<bool>(
    context: context,
    barrierDismissible: false,
    builder: (_) => _TaskProgressDialog(
      poll: poll,
      onCancel: onCancel,
      determinate: determinate,
      title: title,
      failureTitle: failureTitle,
    ),
  );
}

class _TaskProgressDialog extends StatefulWidget {
  const _TaskProgressDialog({
    required this.poll,
    required this.onCancel,
    required this.determinate,
    this.title,
    this.failureTitle,
  });

  final String Function() poll;
  final VoidCallback? onCancel;
  final bool determinate;
  final String? title;
  final String? failureTitle;

  @override
  State<_TaskProgressDialog> createState() => _TaskProgressDialogState();
}

class _TaskProgressDialogState extends State<_TaskProgressDialog> {
  AppLocalizations get l => AppLocalizations.of(context);

  Timer? _timer;
  double _progress = 0;
  String _status = '';
  String _error = '';
  bool _cancelling = false;

  @override
  void initState() {
    super.initState();
    _poll();
    _timer = Timer.periodic(const Duration(milliseconds: 200), (_) => _poll());
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  void _poll() {
    final raw = widget.poll();
    if (raw.isEmpty) {
      return;
    }
    try {
      final map = jsonDecode(raw) as Map<String, dynamic>;
      final state = (map['state'] as num?)?.toInt() ?? 0;
      if (state == 2) {
        _timer?.cancel();
        if (mounted) {
          Navigator.of(context).pop(!_cancelling);
        }
        return;
      }
      if (state == 3) {
        _timer?.cancel();
        if (mounted) {
          setState(() {
            _error = (map['error'] as String?)?.trim() ?? '';
            if (_error.isEmpty) {
              _error = widget.failureTitle ?? l.exportFailedShort;
            }
          });
        }
        return;
      }
      if (!mounted) {
        return;
      }
      setState(() {
        _progress = ((map['progress'] as num?)?.toDouble() ?? 0).clamp(
          0.0,
          1.0,
        );
        final status = (map['status'] as String?)?.trim() ?? '';
        if (!_cancelling && status.isNotEmpty) {
          _status = status;
        }
      });
    } catch (_) {
      // 轮询期间 JSON 不完整/为空时忽略，下一轮再读。
    }
  }

  void _cancel() {
    final cancel = widget.onCancel;
    if (_cancelling || cancel == null) {
      return;
    }
    _cancelling = true;
    setState(() => _status = l.cancelling);
    cancel();
  }

  @override
  Widget build(BuildContext context) {
    final failed = _error.isNotEmpty;
    return PopScope(
      canPop: false,
      child: YinheDialog(
        title: failed
            ? (widget.failureTitle ?? l.exportFailedShort)
            : (widget.title ?? l.exporting),
        width: 400,
        actions: failed
            ? [
                FilledButton(
                  onPressed: () => Navigator.of(context).pop(false),
                  child: Text(l.ok),
                ),
              ]
            : widget.onCancel == null
            ? const <Widget>[]
            : [
                TextButton(
                  onPressed: _cancelling ? null : _cancel,
                  child: Text(l.cancel),
                ),
              ],
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            LinearProgressIndicator(
              value: widget.determinate && !failed ? _progress : null,
            ),
            if (failed || _status.isNotEmpty) ...[
              const SizedBox(height: 12),
              Text(
                failed ? _error : _status,
                style: TextStyle(
                  color: failed ? YinheTheme.danger : YinheTheme.textSecondary,
                  fontSize: 13,
                ),
              ),
            ],
            if (widget.determinate && !failed) ...[
              const SizedBox(height: 4),
              Text(
                '${(_progress * 100).round()}%',
                style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
