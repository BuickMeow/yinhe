import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';

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
    return YinheDialog(
      title: l.opHumanize,
      content: Column(
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
/// `selectionRange`：当前选区 tick 范围，用于变速预估显示。
Future<BatchEditSpec?> showBatchEditDialog(
  BuildContext context, {
  required bool automation,
  (double, double)? selectionRange,
  String Function(double)? formatTick,
}) {
  return showDialog<BatchEditSpec>(
    context: context,
    builder: (_) => _BatchEditDialog(
      automation: automation,
      selectionRange: selectionRange,
      formatTick: formatTick,
    ),
  );
}

class _BatchEditDialog extends StatefulWidget {
  const _BatchEditDialog({
    required this.automation,
    this.selectionRange,
    this.formatTick,
  });

  final bool automation;
  final (double, double)? selectionRange;
  final String Function(double)? formatTick;

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

  /// 仅用于界面预估：支持纯数字、`x` 前缀和 `a/b`。
  double? _stretchFactor() {
    var s = _stretch.text.trim();
    if (s.isEmpty) {
      return 1;
    }
    if (s.startsWith('x') ||
        s.startsWith('X') ||
        s.startsWith('×') ||
        s.startsWith('*')) {
      s = s.substring(1);
    }
    final parts = s.split('/');
    if (parts.length == 2) {
      final a = double.tryParse(parts[0].trim());
      final b = double.tryParse(parts[1].trim());
      if (a == null || b == null || b == 0) {
        return null;
      }
      return a / b;
    }
    return double.tryParse(s);
  }

  /// 变速预估：`原终点 → 新终点`（起点不变）。
  String? _stretchPreview() {
    final range = widget.selectionRange;
    final format = widget.formatTick;
    if (range == null || format == null) {
      return null;
    }
    final factor = _stretchFactor();
    if (factor == null || !factor.isFinite || factor <= 0) {
      return null;
    }
    final (t0, t1) = range;
    final newEnd = t0 + (t1 - t0) * factor;
    return '${format(t1)} → ${format(newEnd)}';
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final preview = widget.automation ? null : _stretchPreview();
    return YinheDialog(
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
                        onChanged: () => setState(() {}),
                      ),
                    ),
                ],
              ),
              if (preview != null) ...[
                const SizedBox(height: 6),
                Text(
                  preview,
                  style: TextStyle(color: YinheTheme.textMuted, fontSize: 11),
                ),
              ],
            ],
          );
        },
      ),
      width: 340,
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
  const _ExprField({
    required this.label,
    required this.controller,
    this.onChanged,
  });

  final String label;
  final TextEditingController controller;
  final VoidCallback? onChanged;

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
          onChanged: onChanged == null ? null : (_) => onChanged!(),
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
