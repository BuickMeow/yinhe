import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../core/bar_beat.dart';
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
  int _timing = 8;
  int _velocity = 8;
  int _gate = 8;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return YinheDialog(
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          _NumberStepper(
            label: l.humanizeTiming,
            value: _timing,
            min: 0,
            max: 120,
            suffix: 'tick',
            onChanged: (v) => setState(() => _timing = v),
          ),
          const SizedBox(height: 6),
          _NumberStepper(
            label: l.humanizeVelocity,
            value: _velocity,
            min: 0,
            max: 64,
            onChanged: (v) => setState(() => _velocity = v),
          ),
          const SizedBox(height: 6),
          _NumberStepper(
            label: l.humanizeGate,
            value: _gate,
            min: 0,
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
            HumanizeSpec(timing: _timing, velocity: _velocity, gate: _gate),
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
/// `grid` / `ppq`：把长度显示成 `小节.拍.tick`，并解析用户直接输入的目标长度。
Future<BatchEditSpec?> showBatchEditDialog(
  BuildContext context, {
  required bool automation,
  (double, double)? selectionRange,
  BarBeatGrid? grid,
  int ppq = 480,
}) {
  return showDialog<BatchEditSpec>(
    context: context,
    builder: (_) => _BatchEditDialog(
      automation: automation,
      selectionRange: selectionRange,
      grid: grid,
      ppq: ppq,
    ),
  );
}

class _BatchEditDialog extends StatefulWidget {
  const _BatchEditDialog({
    required this.automation,
    this.selectionRange,
    this.grid,
    this.ppq = 480,
  });

  final bool automation;
  final (double, double)? selectionRange;
  final BarBeatGrid? grid;
  final int ppq;

  @override
  State<_BatchEditDialog> createState() => _BatchEditDialogState();
}

class _BatchEditDialogState extends State<_BatchEditDialog> {
  final _value = TextEditingController();
  final _gate = TextEditingController();
  final _key = TextEditingController();
  final _tick = TextEditingController();
  final _stretch = TextEditingController();
  final _length = TextEditingController();

  @override
  void dispose() {
    _value.dispose();
    _gate.dispose();
    _key.dispose();
    _tick.dispose();
    _stretch.dispose();
    _length.dispose();
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

  double get _selectionStart => widget.selectionRange?.$1 ?? 0;

  /// 选区起点所在段的小节长 / 拍长（tick）。
  (int, int) _barBeatTicks() {
    final grid = widget.grid;
    if (grid != null) {
      return grid.ticksOfSegment(grid.segmentIndexAt(_selectionStart.round()));
    }
    final ppq = widget.ppq > 0 ? widget.ppq : 480;
    return (ppq * 4, ppq);
  }

  /// 时长 → `小节.拍.tick`（从 0 开始，不是绝对位置）。
  String _formatLength(double ticks) {
    final grid = widget.grid;
    if (grid != null) {
      return grid.formatDuration(ticks, atTick: _selectionStart.round());
    }
    if (ticks <= 0) {
      return '0.0.000';
    }
    final (measure, beat) = _barBeatTicks();
    final t = ticks.floor();
    final bars = t ~/ measure;
    final rem = t % measure;
    final beats = rem ~/ beat;
    final rest = rem % beat;
    return '$bars.$beats.${rest.toString().padLeft(3, '0')}';
  }

  /// 解析目标长度：`4`=4 小节，`4.2`=4 小节 2 拍，`4.2.120`=再 120 tick。
  ///
  /// 空串返回 null（表示不按长度，改用变速倍率）。
  int? _targetTicks() {
    final text = _length.text.trim();
    if (text.isEmpty) {
      return null;
    }
    final (measure, beat) = _barBeatTicks();
    final parts = text.split('.');
    if (parts.length == 1) {
      final bars = double.tryParse(text);
      if (bars == null || !bars.isFinite || bars <= 0) {
        return null;
      }
      return (bars * measure).round();
    }
    if (parts.length > 3) {
      return null;
    }
    final bars = int.tryParse(parts[0]);
    final beats = int.tryParse(parts[1]);
    final ticks = parts.length == 3 ? int.tryParse(parts[2]) : 0;
    if (bars == null || beats == null || ticks == null) {
      return null;
    }
    if (bars < 0 || beats < 0 || ticks < 0) {
      return null;
    }
    return bars * measure + beats * beat + ticks;
  }

  bool get _targetInvalid {
    if (_length.text.trim().isEmpty) {
      return false;
    }
    final target = _targetTicks();
    return target == null || target <= 0;
  }

  /// 提交给 Rust 的变速表达式：目标长度优先，否则用倍率输入。
  String _effectiveStretch() {
    final target = _targetTicks();
    final range = widget.selectionRange;
    if (target != null && target > 0 && range != null) {
      final current = (range.$2 - range.$1).round();
      if (current > 0) {
        return '$target/$current';
      }
    }
    return _stretch.text;
  }

  /// 变速预估：显示的是**长度**（当前 → 目标），不是结束位置。
  String? _stretchPreview() {
    final range = widget.selectionRange;
    if (range == null) {
      return null;
    }
    final len = range.$2 - range.$1;
    if (len <= 0) {
      return null;
    }
    final current = _formatLength(len);
    final text = _length.text.trim();
    if (text.isNotEmpty) {
      final target = _targetTicks();
      if (target == null || target <= 0) {
        return current;
      }
      return '$current → ${_formatLength(target.toDouble())}';
    }
    final factor = _stretchFactor();
    if (factor == null || !factor.isFinite || factor <= 0) {
      return current;
    }
    return '$current → ${_formatLength(len * factor)}';
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
                  if (!widget.automation) ...[
                    SizedBox(
                      width: w,
                      child: _ExprField(
                        label: l.batchStretch,
                        controller: _stretch,
                        onChanged: () => setState(() {}),
                      ),
                    ),
                    SizedBox(
                      width: w,
                      child: _ExprField(
                        label: l.batchLength,
                        controller: _length,
                        hintText: '4.0.000',
                        errorText: _targetInvalid ? l.batchLengthInvalid : null,
                        onChanged: () => setState(() {}),
                      ),
                    ),
                  ],
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
          onPressed: (widget.automation || !_targetInvalid)
              ? () => Navigator.of(context).pop(
                  BatchEditSpec(
                    value: _value.text,
                    gate: widget.automation ? '' : _gate.text,
                    key: widget.automation ? '' : _key.text,
                    tick: _tick.text,
                    stretch: widget.automation ? '' : _effectiveStretch(),
                  ),
                )
              : null,
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
    this.hintText,
    this.errorText,
  });

  final String label;
  final TextEditingController controller;
  final VoidCallback? onChanged;
  final String? hintText;
  final String? errorText;

  @override
  Widget build(BuildContext context) {
    return TextField(
      controller: controller,
      autocorrect: false,
      enableSuggestions: false,
      keyboardType: TextInputType.text,
      style: const TextStyle(fontSize: 14),
      onChanged: onChanged == null ? null : (_) => onChanged!(),
      decoration: dialogInputDecoration(
        label,
        hintText: hintText,
        errorText: errorText,
      ),
    );
  }
}

class _NumberStepper extends StatefulWidget {
  const _NumberStepper({
    required this.label,
    required this.value,
    required this.min,
    required this.max,
    required this.onChanged,
    this.suffix = '',
  });

  final String label;
  final int value;
  final int min;
  final int max;
  final String suffix;
  final ValueChanged<int> onChanged;

  @override
  State<_NumberStepper> createState() => _NumberStepperState();
}

class _NumberStepperState extends State<_NumberStepper> {
  late final TextEditingController _text = TextEditingController(
    text: '${widget.value}',
  );

  @override
  void didUpdateWidget(covariant _NumberStepper oldWidget) {
    super.didUpdateWidget(oldWidget);
    final next = '${widget.value}';
    if (widget.value != oldWidget.value && _text.text != next) {
      _text.text = next;
    }
  }

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  void _commitText(String raw) {
    final parsed = int.tryParse(raw.trim());
    if (parsed == null) {
      return;
    }
    final value = parsed.clamp(widget.min, widget.max);
    if (value != widget.value) {
      widget.onChanged(value);
    }
  }

  void _step(int delta) {
    final next = (widget.value + delta).clamp(widget.min, widget.max);
    if (next != widget.value) {
      widget.onChanged(next);
    }
  }

  @override
  Widget build(BuildContext context) {
    final canDec = widget.value > widget.min;
    final canInc = widget.value < widget.max;
    return Row(
      children: [
        Expanded(
          child: Text(
            widget.label,
            style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
          ),
        ),
        IconButton(
          onPressed: canDec ? () => _step(-1) : null,
          icon: const Icon(Icons.remove, size: 18),
          visualDensity: VisualDensity.compact,
          constraints: const BoxConstraints.tightFor(width: 32, height: 32),
          padding: EdgeInsets.zero,
        ),
        SizedBox(
          width: 56,
          child: TextField(
            controller: _text,
            textAlign: TextAlign.center,
            keyboardType: TextInputType.number,
            inputFormatters: [FilteringTextInputFormatter.digitsOnly],
            style: const TextStyle(fontSize: 14),
            decoration: const InputDecoration(
              isDense: true,
              border: OutlineInputBorder(),
              contentPadding: EdgeInsets.symmetric(horizontal: 4, vertical: 6),
            ),
            onChanged: _commitText,
            onSubmitted: (raw) {
              final parsed = int.tryParse(raw.trim());
              final value = (parsed ?? widget.value).clamp(
                widget.min,
                widget.max,
              );
              _text.text = '$value';
              if (value != widget.value) {
                widget.onChanged(value);
              }
            },
          ),
        ),
        IconButton(
          onPressed: canInc ? () => _step(1) : null,
          icon: const Icon(Icons.add, size: 18),
          visualDensity: VisualDensity.compact,
          constraints: const BoxConstraints.tightFor(width: 32, height: 32),
          padding: EdgeInsets.zero,
        ),
        SizedBox(
          width: 32,
          child: Text(
            widget.suffix,
            style: TextStyle(color: YinheTheme.textMuted, fontSize: 11),
          ),
        ),
      ],
    );
  }
}
