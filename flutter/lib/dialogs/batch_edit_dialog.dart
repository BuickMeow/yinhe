import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n/app_localizations.dart';

/// 类人化幅度：时间/力度/时值三路独立随机偏差。
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

/// 批量处理：每个字段一个表达式，留空 = 不修改。
///
/// 表达式与桌面统一：`100`=设值、`+2`=加、`-2`=减、`x1.5`=乘、
/// `/2`=除、`3/7`=分数、`。`=小数点。
class BatchEditSpec {
  const BatchEditSpec({
    this.value = '',
    this.gate = '',
    this.key = '',
    this.tick = '',
    this.stretch = '',
  });

  /// 音符 = 力度；自动化 = 锚点值。
  final String value;
  final String gate;
  final String key;
  final String tick;

  /// 变速倍率表达式：`1.5`、`4/3`、`x2`；空 = 不变。
  final String stretch;
}

/// 弹批量处理对话框；取消返回 null。
///
/// `automation`：只显示「力度/值」和「位置」两项。
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

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return AlertDialog(
      title: Text(l.batchProcess),
      content: LayoutBuilder(
        builder: (context, constraints) {
          final w = (constraints.maxWidth - 12) / 2;
          return Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Wrap(
                spacing: 12,
                runSpacing: 8,
                children: [
                  SizedBox(
                    width: w,
                    child: _ExprField(
                      label: l.batchVelocity,
                      controller: _value,
                    ),
                  ),
                  if (!widget.automation) ...[
                    SizedBox(
                      width: w,
                      child: _ExprField(label: l.batchGate, controller: _gate),
                    ),
                    SizedBox(
                      width: w,
                      child: _ExprField(label: l.batchKey, controller: _key),
                    ),
                  ],
                  SizedBox(
                    width: w,
                    child: _ExprField(label: l.batchTick, controller: _tick),
                  ),
                  if (!widget.automation)
                    SizedBox(
                      width: w,
                      child: _ExprField(
                        label: l.batchStretch,
                        controller: _stretch,
                      ),
                    ),
                ],
              ),
              const SizedBox(height: 10),
              Text(
                l.batchExprHint,
                style: TextStyle(color: YinheTheme.textMuted, fontSize: 11),
              ),
            ],
          );
        },
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(
            BatchEditSpec(
              value: _value.text,
              gate: widget.automation ? '' : _gate.text,
              key: widget.automation ? '' : _key.text,
              tick: _tick.text,
              stretch: widget.automation ? '' : _stretch.text,
            ),
          ),
          child: Text(l.ok),
        ),
      ],
    );
  }
}

/// 一列字段：标签在上、表达式输入在下。
class _ExprField extends StatelessWidget {
  const _ExprField({required this.label, required this.controller});

  final String label;
  final TextEditingController controller;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          label,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(color: YinheTheme.textLabel, fontSize: 11),
        ),
        const SizedBox(height: 3),
        TextField(
          controller: controller,
          autocorrect: false,
          enableSuggestions: false,
          keyboardType: TextInputType.text,
          style: const TextStyle(fontSize: 14),
          decoration: const InputDecoration(
            isDense: true,
            border: OutlineInputBorder(),
            contentPadding: EdgeInsets.symmetric(horizontal: 8, vertical: 8),
          ),
        ),
      ],
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
