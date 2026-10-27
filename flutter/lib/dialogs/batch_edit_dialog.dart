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

/// 弹类人化参数对话框；取消返回 null。
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

/// 批量处理单字段：操作码 + 数值；数值为空 = 不修改。
///
/// 操作码：1=设值 2=加 3=减 4=乘 5=除。
class BatchFieldSpec {
  const BatchFieldSpec(this.op, this.value);

  final int op;
  final double? value;
}

class BatchEditSpec {
  const BatchEditSpec({
    required this.value,
    required this.gate,
    required this.key,
    required this.tick,
    required this.stretchPercent,
  });

  /// 音符 = 力度；自动化 = 锚点值。
  final BatchFieldSpec value;
  final BatchFieldSpec gate;
  final BatchFieldSpec key;
  final BatchFieldSpec tick;
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
  final _value = _FieldController();
  final _gate = _FieldController();
  final _key = _FieldController();
  final _tick = _FieldController();
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

  int _stretchPercent() {
    final text = _stretch.text.trim();
    if (text.isEmpty) {
      return 100;
    }
    return (double.tryParse(text) ?? 100).round();
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
              controller: _value,
              onChanged: () => setState(() {}),
            ),
            if (!widget.automation) ...[
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
            ],
            _BatchFieldRow(
              label: l.batchTick,
              controller: _tick,
              onChanged: () => setState(() {}),
            ),
            if (!widget.automation)
              _PlainNumRow(
                label: l.batchStretch,
                controller: _stretch,
                suffix: '%',
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
              value: _value.spec(),
              gate: widget.automation
                  ? const BatchFieldSpec(1, null)
                  : _gate.spec(),
              key: widget.automation
                  ? const BatchFieldSpec(1, null)
                  : _key.spec(),
              tick: _tick.spec(),
              stretchPercent: widget.automation ? 100 : _stretchPercent(),
            ),
          ),
          child: Text(l.ok),
        ),
      ],
    );
  }
}

class _FieldController {
  int op = 1;
  final TextEditingController text = TextEditingController();

  void dispose() => text.dispose();

  BatchFieldSpec spec() {
    final raw = text.text.trim();
    return BatchFieldSpec(op, raw.isEmpty ? null : double.tryParse(raw));
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
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: Row(
        children: [
          SizedBox(
            width: 64,
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
                DropdownMenuItem(value: 1, child: Text(l.batchOpSet)),
                DropdownMenuItem(value: 2, child: Text(l.batchOpAdd)),
                DropdownMenuItem(value: 3, child: Text(l.batchOpSub)),
                DropdownMenuItem(value: 4, child: Text(l.batchOpMul)),
                DropdownMenuItem(value: 5, child: Text(l.batchOpDiv)),
              ],
              onChanged: (v) {
                controller.op = v ?? 1;
                onChanged();
              },
            ),
          ),
          const SizedBox(width: 8),
          Expanded(child: _numField(controller.text)),
        ],
      ),
    );
  }
}

/// 只有标签 + 数值（变速用）。
class _PlainNumRow extends StatelessWidget {
  const _PlainNumRow({
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
            width: 64,
            child: Text(
              label,
              style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
            ),
          ),
          Expanded(child: _numField(controller, suffix: suffix)),
        ],
      ),
    );
  }
}

Widget _numField(TextEditingController controller, {String suffix = ''}) {
  return TextField(
    controller: controller,
    keyboardType: const TextInputType.numberWithOptions(
      decimal: true,
      signed: true,
    ),
    decoration: InputDecoration(
      isDense: true,
      border: const OutlineInputBorder(),
      contentPadding: const EdgeInsets.symmetric(horizontal: 8, vertical: 8),
      suffixText: suffix.isEmpty ? null : suffix,
    ),
  );
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
