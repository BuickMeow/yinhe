import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import 'frame.dart';

/// 通用数值输入（整数）。返回 null = 取消。
Future<int?> showIntInputDialog(
  BuildContext context, {
  required String title,
  required String label,
  required int min,
  required int max,
  required int initial,
  List<(String, int)> quick = const [],
  String? errorText,
  String confirmText = '',
}) {
  return showDialog<int>(
    context: context,
    builder: (_) => _IntInputDialog(
      title: title,
      label: label,
      min: min,
      max: max,
      initial: initial,
      quick: quick,
      errorText: errorText,
      confirmText: confirmText,
    ),
  );
}

/// 通用数值输入（小数，如 BPM）。返回 null = 取消。
Future<double?> showDecimalInputDialog(
  BuildContext context, {
  required String title,
  required String label,
  required double min,
  required double max,
  required double initial,
  String? errorText,
}) {
  return showDialog<double>(
    context: context,
    builder: (_) => _DecimalInputDialog(
      title: title,
      label: label,
      min: min,
      max: max,
      initial: initial,
      errorText: errorText,
    ),
  );
}

/// 单行文本输入。返回 null = 取消。
Future<String?> showTextInputDialog(
  BuildContext context, {
  required String title,
  required String label,
  String initial = '',
  bool obscure = false,
  String confirmText = '',
}) {
  return showDialog<String>(
    context: context,
    builder: (_) => _TextInputDialog(
      title: title,
      label: label,
      initial: initial,
      obscure: obscure,
      confirmText: confirmText,
    ),
  );
}

/// 拍号输入（如 4/4）。返回 `(分子, 分母)`。
Future<(int, int)?> showTimeSigInputDialog(
  BuildContext context, {
  required String title,
  required String label,
  int numerator = 4,
  int denominator = 4,
}) {
  return showDialog<(int, int)>(
    context: context,
    builder: (_) => _TimeSigInputDialog(
      title: title,
      label: label,
      numerator: numerator,
      denominator: denominator,
    ),
  );
}

/// 调号输入。返回 `(主音 0=C..11=B, 0=大调 1=自然小调)`。
Future<(int, int)?> showKeySigInputDialog(
  BuildContext context, {
  required String title,
  required String rootLabel,
  required String scaleLabel,
  String majorLabel = 'Major',
  String minorLabel = 'Minor',
}) {
  return showDialog<(int, int)>(
    context: context,
    builder: (_) => _KeySigInputDialog(
      title: title,
      rootLabel: rootLabel,
      scaleLabel: scaleLabel,
      majorLabel: majorLabel,
      minorLabel: minorLabel,
    ),
  );
}

/// 音符修改：力度 + 长度（tick）。
Future<(int, int)?> showNoteInputDialog(
  BuildContext context, {
  required String title,
  required String velocityLabel,
  required String lengthLabel,
  required int velocity,
  required int gate,
  String confirmText = '',
}) {
  return showDialog<(int, int)>(
    context: context,
    builder: (_) => _NoteInputDialog(
      title: title,
      velocityLabel: velocityLabel,
      lengthLabel: lengthLabel,
      velocity: velocity,
      gate: gate,
      confirmText: confirmText,
    ),
  );
}

class _IntInputDialog extends StatefulWidget {
  const _IntInputDialog({
    required this.title,
    required this.label,
    required this.min,
    required this.max,
    required this.initial,
    required this.quick,
    this.errorText,
    this.confirmText = '',
  });

  final String title;
  final String label;
  final int min;
  final int max;
  final int initial;
  final List<(String, int)> quick;
  final String? errorText;
  final String confirmText;

  @override
  State<_IntInputDialog> createState() => _IntInputDialogState();
}

class _IntInputDialogState extends State<_IntInputDialog> {
  late final TextEditingController _c = TextEditingController(
    text: '${widget.initial}',
  );
  String? _error;

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  void _confirm() {
    final v = int.tryParse(_c.text.trim());
    if (v == null || v < widget.min || v > widget.max) {
      setState(
        () => _error = widget.errorText ?? '${widget.min}–${widget.max}',
      );
      return;
    }
    Navigator.of(context).pop(v);
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return YinheDialog(
      title: widget.title,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(
          onPressed: _confirm,
          child: Text(widget.confirmText.isEmpty ? l.ok : widget.confirmText),
        ),
      ],
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            controller: _c,
            autofocus: true,
            keyboardType: TextInputType.number,
            style: const TextStyle(fontSize: 14),
            decoration: dialogInputDecoration(widget.label, errorText: _error),
            onSubmitted: (_) => _confirm(),
          ),
          if (widget.quick.isNotEmpty) ...[
            const SizedBox(height: 10),
            Wrap(
              spacing: 8,
              children: [
                for (final (label, value) in widget.quick)
                  OutlinedButton(
                    onPressed: () => setState(() => _c.text = '$value'),
                    child: Text('$label（$value）'),
                  ),
              ],
            ),
          ],
        ],
      ),
    );
  }
}

class _DecimalInputDialog extends StatefulWidget {
  const _DecimalInputDialog({
    required this.title,
    required this.label,
    required this.min,
    required this.max,
    required this.initial,
    this.errorText,
  });

  final String title;
  final String label;
  final double min;
  final double max;
  final double initial;
  final String? errorText;

  @override
  State<_DecimalInputDialog> createState() => _DecimalInputDialogState();
}

class _DecimalInputDialogState extends State<_DecimalInputDialog> {
  late final TextEditingController _c = TextEditingController(
    text: widget.initial == widget.initial.roundToDouble()
        ? '${widget.initial.round()}'
        : '${widget.initial}',
  );
  String? _error;

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  void _confirm() {
    final v = double.tryParse(_c.text.trim());
    if (v == null || v < widget.min || v > widget.max) {
      setState(
        () => _error = widget.errorText ?? '${widget.min}–${widget.max}',
      );
      return;
    }
    Navigator.of(context).pop(v);
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return YinheDialog(
      title: widget.title,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(onPressed: _confirm, child: Text(l.save)),
      ],
      content: TextField(
        controller: _c,
        autofocus: true,
        keyboardType: const TextInputType.numberWithOptions(decimal: true),
        style: const TextStyle(fontSize: 14),
        decoration: dialogInputDecoration(widget.label, errorText: _error),
        onSubmitted: (_) => _confirm(),
      ),
    );
  }
}

class _TextInputDialog extends StatefulWidget {
  const _TextInputDialog({
    required this.title,
    required this.label,
    required this.initial,
    required this.obscure,
    this.confirmText = '',
  });

  final String title;
  final String label;
  final String initial;
  final bool obscure;
  final String confirmText;

  @override
  State<_TextInputDialog> createState() => _TextInputDialogState();
}

class _TextInputDialogState extends State<_TextInputDialog> {
  late final TextEditingController _c = TextEditingController(
    text: widget.initial,
  );

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return YinheDialog(
      title: widget.title,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_c.text),
          child: Text(widget.confirmText.isEmpty ? l.ok : widget.confirmText),
        ),
      ],
      content: TextField(
        controller: _c,
        autofocus: true,
        obscureText: widget.obscure,
        style: const TextStyle(fontSize: 14),
        decoration: dialogInputDecoration(widget.label),
        onSubmitted: (v) => Navigator.of(context).pop(v),
      ),
    );
  }
}

class _TimeSigInputDialog extends StatefulWidget {
  const _TimeSigInputDialog({
    required this.title,
    required this.label,
    required this.numerator,
    required this.denominator,
  });

  final String title;
  final String label;
  final int numerator;
  final int denominator;

  @override
  State<_TimeSigInputDialog> createState() => _TimeSigInputDialogState();
}

class _TimeSigInputDialogState extends State<_TimeSigInputDialog> {
  late final TextEditingController _c = TextEditingController(
    text: '${widget.numerator}/${widget.denominator}',
  );
  String? _error;

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  void _confirm() {
    final parts = _c.text.trim().split('/');
    final num = parts.length == 2 ? int.tryParse(parts[0].trim()) : null;
    final den = parts.length == 2 ? int.tryParse(parts[1].trim()) : null;
    if (num == null ||
        den == null ||
        num < 1 ||
        num > 32 ||
        den < 1 ||
        den > 32 ||
        (den & (den - 1)) != 0) {
      setState(() => _error = '4/4, 3/4, 6/8 …');
      return;
    }
    Navigator.of(context).pop((num, den));
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return YinheDialog(
      title: widget.title,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(onPressed: _confirm, child: Text(l.insert)),
      ],
      content: TextField(
        controller: _c,
        autofocus: true,
        style: const TextStyle(fontSize: 14),
        decoration: dialogInputDecoration(widget.label, errorText: _error),
        onSubmitted: (_) => _confirm(),
      ),
    );
  }
}

class _KeySigInputDialog extends StatefulWidget {
  const _KeySigInputDialog({
    required this.title,
    required this.rootLabel,
    required this.scaleLabel,
    required this.majorLabel,
    required this.minorLabel,
  });

  final String title;
  final String rootLabel;
  final String scaleLabel;
  final String majorLabel;
  final String minorLabel;

  @override
  State<_KeySigInputDialog> createState() => _KeySigInputDialogState();
}

class _KeySigInputDialogState extends State<_KeySigInputDialog> {
  static const _roots = [
    'C',
    'C#',
    'D',
    'D#',
    'E',
    'F',
    'F#',
    'G',
    'G#',
    'A',
    'A#',
    'B',
  ];
  int _root = 0;
  int _scale = 0;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return YinheDialog(
      title: widget.title,
      width: 380,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop((_root, _scale)),
          child: Text(l.insert),
        ),
      ],
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          DropdownButtonFormField<int>(
            initialValue: _root,
            decoration: dialogInputDecoration(widget.rootLabel),
            items: [
              for (var i = 0; i < _roots.length; i++)
                DropdownMenuItem(value: i, child: Text(_roots[i])),
            ],
            onChanged: (v) => setState(() => _root = v ?? _root),
          ),
          const SizedBox(height: 12),
          DropdownButtonFormField<int>(
            initialValue: _scale,
            decoration: dialogInputDecoration(widget.scaleLabel),
            items: [
              DropdownMenuItem(value: 0, child: Text(widget.majorLabel)),
              DropdownMenuItem(value: 1, child: Text(widget.minorLabel)),
            ],
            onChanged: (v) => setState(() => _scale = v ?? _scale),
          ),
        ],
      ),
    );
  }
}

class _NoteInputDialog extends StatefulWidget {
  const _NoteInputDialog({
    required this.title,
    required this.velocityLabel,
    required this.lengthLabel,
    required this.velocity,
    required this.gate,
    this.confirmText = '',
  });

  final String title;
  final String velocityLabel;
  final String lengthLabel;
  final int velocity;
  final int gate;
  final String confirmText;

  @override
  State<_NoteInputDialog> createState() => _NoteInputDialogState();
}

class _NoteInputDialogState extends State<_NoteInputDialog> {
  late final TextEditingController _vel = TextEditingController(
    text: '${widget.velocity}',
  );
  late final TextEditingController _gate = TextEditingController(
    text: '${widget.gate}',
  );
  String? _error;

  @override
  void dispose() {
    _vel.dispose();
    _gate.dispose();
    super.dispose();
  }

  void _confirm() {
    final vel = int.tryParse(_vel.text.trim());
    final gate = int.tryParse(_gate.text.trim());
    if (vel == null || vel < 1 || vel > 127 || gate == null || gate < 1) {
      setState(() => _error = '1–127 / ≥ 1');
      return;
    }
    Navigator.of(context).pop((vel, gate));
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return YinheDialog(
      title: widget.title,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(
          onPressed: _confirm,
          child: Text(widget.confirmText.isEmpty ? l.save : widget.confirmText),
        ),
      ],
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            controller: _vel,
            autofocus: true,
            keyboardType: TextInputType.number,
            style: const TextStyle(fontSize: 14),
            decoration: dialogInputDecoration(widget.velocityLabel),
          ),
          const SizedBox(height: 12),
          TextField(
            controller: _gate,
            keyboardType: TextInputType.number,
            style: const TextStyle(fontSize: 14),
            decoration: dialogInputDecoration(
              widget.lengthLabel,
              errorText: _error,
            ),
            onSubmitted: (_) => _confirm(),
          ),
        ],
      ),
    );
  }
}
