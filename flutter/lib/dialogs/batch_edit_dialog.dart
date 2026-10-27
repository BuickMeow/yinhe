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

/// 批量处理：一个字段的操作（0=不改 1=设值 2=加 3=减 4=乘 5=除）。
class BatchFieldSpec {
  const BatchFieldSpec(this.op, this.value);

  final int op;
  final double value;

  bool get active => op != 0;
}

class BatchEditSpec {
  const BatchEditSpec({
    required this.velocity,
    required this.gate,
    required this.key,
    required this.tick,
    required this.stretchPercent,
  });

  final BatchFieldSpec velocity;
  final BatchFieldSpec gate;
  final BatchFieldSpec key;
  final BatchFieldSpec tick;
  final int stretchPercent;
}

/// 弹批量处理对话框；取消返回 null。
Future<BatchEditSpec?> showBatchEditDialog(BuildContext context) {
  return showDialog<BatchEditSpec>(
    context: context,
    builder: (_) => const _BatchEditDialog(),
  );
}

class _BatchEditDialog extends StatefulWidget {
  const _BatchEditDialog();

  @override
  State<_BatchEditDialog> createState() => _BatchEditDialogState();
}

class _BatchEditDialogState extends State<_BatchEditDialog> {
  final _velocity = _FieldController();
  final _gate = _FieldController();
  final _key = _FieldController();
  final _tick = _FieldController();
  double _stretch = 100;

  @override
  void dispose() {
    _velocity.dispose();
    _gate.dispose();
    _key.dispose();
    _tick.dispose();
    super.dispose();
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
            _BatchFieldRow(
              label: l.batchVelocity,
              controller: _velocity,
              onChanged: () => setState(() {}),
            ),
            _BatchFieldRow(
              label: l.batchGate,
              controller: _gate,
              onChanged: () => setState(() {}),
            ),
            _BatchFieldRow(
              label: l.batchKey,
              controller: _key,
              onChanged: () => setState(() {}),
            ),
            _BatchFieldRow(
              label: l.batchTick,
              controller: _tick,
              onChanged: () => setState(() {}),
            ),
            const SizedBox(height: 12),
            _LabeledSlider(
              label: l.batchStretch,
              value: _stretch,
              min: 25,
              max: 400,
              divisions: 75,
              suffix: '%',
              onChanged: (v) => setState(() => _stretch = v),
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
            BatchEditSpec(
              velocity: _velocity.spec(),
              gate: _gate.spec(),
              key: _key.spec(),
              tick: _tick.spec(),
              stretchPercent: _stretch.round(),
            ),
          ),
          child: Text(l.ok),
        ),
      ],
    );
  }
}

class _FieldController {
  int op = 0;
  final TextEditingController text = TextEditingController();

  void dispose() => text.dispose();

  BatchFieldSpec spec() {
    final v = double.tryParse(text.text.trim()) ?? 0;
    return BatchFieldSpec(op == 0 ? 0 : op, v);
  }
}

class _BatchFieldRow extends StatelessWidget {
  const _BatchFieldRow({
    required this.label,
    required this.controller,
    required this.onChanged,
  });

  final String label;
  final _FieldController controller;
  final VoidCallback onChanged;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final active = controller.op != 0;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: Row(
        children: [
          SizedBox(
            width: 56,
            child: Text(
              label,
              style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
            ),
          ),
          DropdownButtonHideUnderline(
            child: DropdownButton<int>(
              value: controller.op,
              isDense: true,
              style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
              items: [
                DropdownMenuItem(value: 0, child: Text(l.batchOpNone)),
                DropdownMenuItem(value: 1, child: Text(l.batchOpSet)),
                DropdownMenuItem(value: 2, child: Text(l.batchOpAdd)),
                DropdownMenuItem(value: 3, child: Text(l.batchOpSub)),
                DropdownMenuItem(value: 4, child: Text(l.batchOpMul)),
                DropdownMenuItem(value: 5, child: Text(l.batchOpDiv)),
              ],
              onChanged: (v) {
                controller.op = v ?? 0;
                onChanged();
              },
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: TextField(
              controller: controller.text,
              enabled: active,
              keyboardType: const TextInputType.numberWithOptions(
                decimal: true,
                signed: true,
              ),
              decoration: const InputDecoration(
                isDense: true,
                border: OutlineInputBorder(),
                contentPadding: EdgeInsets.symmetric(
                  horizontal: 8,
                  vertical: 8,
                ),
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
    this.min = 0,
    this.max = 1,
    this.divisions,
    this.suffix = '',
  });

  final String label;
  final double value;
  final double min;
  final double max;
  final int? divisions;
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
          value: value.clamp(min, max),
          min: min,
          max: max,
          divisions: divisions,
          onChanged: onChanged,
        ),
      ],
    );
  }
}
