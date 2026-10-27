import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import 'frame.dart';

/// 长按自动化锚点：直接编辑 tick 与值。
///
/// 返回 `(tick, 原始显示值)`（未归一化）；取消返回 null。
Future<(int, double)?> showAutomationAnchorDialog(
  BuildContext context, {
  required String laneName,
  required int tick,
  required double value,
  required double max,
  required bool isTempo,
}) {
  return showDialog<(int, double)>(
    context: context,
    builder: (_) => _AutomationAnchorDialog(
      laneName: laneName,
      tick: tick,
      value: value,
      max: max,
      isTempo: isTempo,
    ),
  );
}

class _AutomationAnchorDialog extends StatefulWidget {
  const _AutomationAnchorDialog({
    required this.laneName,
    required this.tick,
    required this.value,
    required this.max,
    required this.isTempo,
  });

  final String laneName;
  final int tick;
  final double value;
  final double max;
  final bool isTempo;

  @override
  State<_AutomationAnchorDialog> createState() =>
      _AutomationAnchorDialogState();
}

class _AutomationAnchorDialogState extends State<_AutomationAnchorDialog> {
  late final TextEditingController _tick = TextEditingController(
    text: '${widget.tick}',
  );
  late final TextEditingController _value = TextEditingController(
    text: _formatValue(widget.value),
  );
  String? _error;

  static String _formatValue(double v) {
    // 最多两位小数，去掉多余 0（Tempo 类似 120.37 也能保留）。
    final text = v.toStringAsFixed(2);
    return text.replaceFirst(RegExp(r'\.?0+$'), '');
  }

  @override
  void dispose() {
    _tick.dispose();
    _value.dispose();
    super.dispose();
  }

  void _submit() {
    final tick = int.tryParse(_tick.text.trim());
    final value = double.tryParse(_value.text.trim());
    if (tick == null || tick < 0) {
      setState(() => _error = 'Tick ≥ 0');
      return;
    }
    if (value == null) {
      setState(() => _error = null);
      return;
    }
    final min = widget.isTempo ? 1.0 : 0.0;
    if (value < min || value > widget.max) {
      setState(
        () => _error =
            '${min.toStringAsFixed(0)} ~ ${widget.max.toStringAsFixed(0)}',
      );
      return;
    }
    Navigator.of(context).pop((tick, value));
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return YinheDialog(
      title: widget.laneName,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(onPressed: _submit, child: Text(l.ok)),
      ],
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          TextField(
            controller: _tick,
            keyboardType: TextInputType.number,
            style: const TextStyle(fontSize: 14),
            decoration: dialogInputDecoration('Tick'),
          ),
          const SizedBox(height: 12),
          TextField(
            controller: _value,
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
            style: const TextStyle(fontSize: 14),
            decoration: dialogInputDecoration(l.value, errorText: _error),
            onSubmitted: (_) => _submit(),
          ),
        ],
      ),
    );
  }
}
