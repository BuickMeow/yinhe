import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n/app_localizations.dart';

/// 人性化幅度：时间/力度/时值三路独立随机偏差。
class HumanizeSpec {
  const HumanizeSpec({
    required this.timing,
    required this.velocity,
    required this.gate,
  });

  final int timing;
  final int velocity;
  final int gate;
}

/// 弹人性化参数对话框；取消返回 null。
Future<HumanizeSpec?> showHumanizeDialog(BuildContext context) {
  return showDialog<HumanizeSpec>(
    context: context,
    builder: (_) => const _HumanizeDialog(),
  );
}

class _HumanizeDialog extends StatefulWidget {
  const _HumanizeDialog();

  @override
  State<_HumanizeDialog> createState() => _HumanizeDialogState();
}

class _HumanizeDialogState extends State<_HumanizeDialog> {
  double _timing = 8;
  double _velocity = 8;
  double _gate = 8;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return AlertDialog(
      title: Text(l.opHumanize),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            _LabeledSlider(
              label: l.humanizeTiming,
              value: _timing,
              max: 120,
              suffix: 'tick',
              onChanged: (v) => setState(() => _timing = v),
            ),
            _LabeledSlider(
              label: l.humanizeVelocity,
              value: _velocity,
              max: 64,
              onChanged: (v) => setState(() => _velocity = v),
            ),
            _LabeledSlider(
              label: l.humanizeGate,
              value: _gate,
              max: 120,
              suffix: 'tick',
              onChanged: (v) => setState(() => _gate = v),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(
            HumanizeSpec(
              timing: _timing.round(),
              velocity: _velocity.round(),
              gate: _gate.round(),
            ),
          ),
          child: Text(l.ok),
        ),
      ],
    );
  }
}

/// 批量处理：每个字段只做“设值”，留空 = 不修改。
class BatchEditSpec {
  const BatchEditSpec({
    this.value,
    this.gate,
    this.key,
    this.tick,
    this.stretchPercent = 100,
  });

  /// 音符 = 力度；自动化 = 锚点值。
  final double? value;
  final double? gate;
  final double? key;
  final double? tick;
  final int stretchPercent;
}

/// 弹批量处理对话框；取消返回 null。
///
/// `automation`：只显示「力度/值」和「位置」两行。
Future<BatchEditSpec?> showBatchEditDialog(
  BuildContext context, {
  required bool automation,
}) {
  return showDialog<BatchEditSpec>(
    context: context,
    builder: (_) => _BatchEditDialog(automation: automation),
  );
}

class _BatchEditDialog extends StatefulWidget {
  const _BatchEditDialog({required this.automation});

  final bool automation;

  @override
  State<_BatchEditDialog> createState() => _BatchEditDialogState();
}

class _BatchEditDialogState extends State<_BatchEditDialog> {
  final _value = TextEditingController();
  final _gate = TextEditingController();
  final _key = TextEditingController();
  final _tick = TextEditingController();
  final _stretch = TextEditingController();

  @override
  void dispose() {
    _value.dispose();
    _gate.dispose();
    _key.dispose();
    _tick.dispose();
    _stretch.dispose();
    super.dispose();
  }

  double? _parse(TextEditingController c) {
    final text = c.text.trim();
    if (text.isEmpty) {
      return null;
    }
    return double.tryParse(text);
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return AlertDialog(
      title: Text(l.batchProcess),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            _NumRow(label: l.batchVelocity, controller: _value),
            if (!widget.automation) ...[
              _NumRow(label: l.batchGate, controller: _gate),
              _NumRow(label: l.batchKey, controller: _key),
            ],
            _NumRow(label: l.batchTick, controller: _tick),
            if (!widget.automation)
              _NumRow(label: l.batchStretch, controller: _stretch, suffix: '%'),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(
            BatchEditSpec(
              value: _parse(_value),
              gate: widget.automation ? null : _parse(_gate),
              key: widget.automation ? null : _parse(_key),
              tick: _parse(_tick),
              stretchPercent: widget.automation
                  ? 100
                  : (_parse(_stretch)?.round() ?? 100),
            ),
          ),
          child: Text(l.ok),
        ),
      ],
    );
  }
}

/// 一行：[标签][数值输入]，固定“设值”。
class _NumRow extends StatelessWidget {
  const _NumRow({
    required this.label,
    required this.controller,
    this.suffix = '',
  });

  final String label;
  final TextEditingController controller;
  final String suffix;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          SizedBox(
            width: 72,
            child: Text(
              label,
              style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
            ),
          ),
          Expanded(
            child: TextField(
              controller: controller,
              keyboardType: const TextInputType.numberWithOptions(
                decimal: true,
                signed: true,
              ),
              decoration: InputDecoration(
                isDense: true,
                border: const OutlineInputBorder(),
                contentPadding: const EdgeInsets.symmetric(
                  horizontal: 8,
                  vertical: 8,
                ),
                suffixText: suffix.isEmpty ? null : suffix,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _LabeledSlider extends StatelessWidget {
  const _LabeledSlider({
    required this.label,
    required this.value,
    required this.onChanged,
    this.max = 1,
    this.suffix = '',
  });

  final String label;
  final double value;
  final double max;
  final String suffix;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                label,
                style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
              ),
            ),
            Text(
              '${value.round()}$suffix',
              style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
            ),
          ],
        ),
        Slider(
          value: value.clamp(0, max),
          min: 0,
          max: max,
          onChanged: onChanged,
        ),
      ],
    );
  }
}
