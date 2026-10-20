import 'package:flutter/material.dart';

import 'theme.dart';
import 'l10n.dart';

/// 本工程默认 BPM / 拍号编辑对话框。
///
/// 这两个值只在 conductor 没有原生 Tempo/拍号事件时生效：
/// 原生事件优先级更高，可以完全代替它们。

/// 弹出「本工程默认 BPM」编辑框；返回保存的 BPM（取消返回 null）。
Future<double?> showDefaultBpmDialog(BuildContext context, double initial) {
  return showDialog<double>(
    context: context,
    builder: (_) => _BpmDialog(initial: initial),
  );
}

/// 弹出「本工程默认拍号」编辑框；返回 `(分子, 分母)`（取消返回 null）。
Future<(int, int)?> showDefaultTimeSigDialog(
  BuildContext context,
  int num,
  int den,
) {
  return showDialog<(int, int)>(
    context: context,
    builder: (_) => _TimeSigDialog(num: num, den: den),
  );
}

class _BpmDialog extends StatefulWidget {
  const _BpmDialog({required this.initial});

  final double initial;

  @override
  State<_BpmDialog> createState() => _BpmDialogState();
}

class _BpmDialogState extends State<_BpmDialog> {
  late final TextEditingController _bpm = TextEditingController(
    text: _fmt(widget.initial),
  );
  String? _error;

  static String _fmt(double v) =>
      v == v.roundToDouble() ? '${v.round()}' : v.toStringAsFixed(1);

  @override
  void dispose() {
    _bpm.dispose();
    super.dispose();
  }

  void _save() {
    final v = double.tryParse(_bpm.text.trim());
    if (v == null || v < 0 || v > 60000000) {
      setState(() => _error = tr('BPM 需为 0–60000000'));
      return;
    }
    Navigator.of(context).pop(v);
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: Text(tr('本工程默认 BPM'), style: TextStyle(fontSize: 18)),
      content: SizedBox(
        width: 320,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // 顶部留白：浮动 label 会超出输入框上缘，避免被裁掉一半。
            const SizedBox(height: 10),
            TextField(
              controller: _bpm,
              autofocus: true,
              keyboardType: TextInputType.number,
              style: const TextStyle(fontSize: 14),
              decoration: InputDecoration(
                labelText: tr('BPM（0–60000000）'),
                border: OutlineInputBorder(),
                isDense: true,
              ),
              onSubmitted: (_) => _save(),
            ),
            if (_error != null)
              Padding(
                padding: const EdgeInsets.only(top: 10),
                child: Text(
                  _error!,
                  style: TextStyle(color: YinheTheme.danger, fontSize: 12),
                ),
              ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(tr('取消')),
        ),
        FilledButton(onPressed: _save, child: Text(tr('保存'))),
      ],
    );
  }
}

class _TimeSigDialog extends StatefulWidget {
  const _TimeSigDialog({required this.num, required this.den});

  final int num;
  final int den;

  @override
  State<_TimeSigDialog> createState() => _TimeSigDialogState();
}

class _TimeSigDialogState extends State<_TimeSigDialog> {
  late final TextEditingController _sig = TextEditingController(
    text: '${widget.num}/${widget.den}',
  );
  String? _error;

  @override
  void dispose() {
    _sig.dispose();
    super.dispose();
  }

  void _save() {
    final parsed = _parseTimeSig(_sig.text.trim());
    if (parsed == null) {
      setState(() => _error = '格式如 4/4、3/4、6/8（分子 1–32，分母 1–32 的 2 次幂）');
      return;
    }
    Navigator.of(context).pop(parsed);
  }

  static (int, int)? _parseTimeSig(String s) {
    final parts = s.split('/');
    if (parts.length != 2) {
      return null;
    }
    final num = int.tryParse(parts[0].trim());
    final den = int.tryParse(parts[1].trim());
    if (num == null || den == null) {
      return null;
    }
    if (num < 1 || num > 32) {
      return null;
    }
    if (den < 1 || den > 32 || (den & (den - 1)) != 0) {
      return null;
    }
    return (num, den);
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: Text(tr('本工程默认拍号'), style: TextStyle(fontSize: 18)),
      content: SizedBox(
        width: 320,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const SizedBox(height: 10),
            TextField(
              controller: _sig,
              autofocus: true,
              keyboardType: TextInputType.text,
              style: const TextStyle(fontSize: 14),
              decoration: InputDecoration(
                labelText: tr('拍号（如 4/4）'),
                border: OutlineInputBorder(),
                isDense: true,
              ),
              onSubmitted: (_) => _save(),
            ),
            if (_error != null)
              Padding(
                padding: const EdgeInsets.only(top: 10),
                child: Text(
                  _error!,
                  style: TextStyle(color: YinheTheme.danger, fontSize: 12),
                ),
              ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(tr('取消')),
        ),
        FilledButton(onPressed: _save, child: Text(tr('保存'))),
      ],
    );
  }
}
