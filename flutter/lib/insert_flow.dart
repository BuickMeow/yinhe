import 'package:flutter/material.dart';

import 'l10n.dart';
import 'theme.dart';

/// 插入目标描述。
class InsertChoice {
  const InsertChoice({
    required this.kind,
    required this.number,
    required this.label,
    this.custom = false,
  });

  /// 0=Tempo、1=PB、2=CC、3=RPN。
  final int kind;

  /// CC 号 / RPN 号（Tempo/PB 忽略）。
  final int number;
  final String label;

  /// 自定义号：先弹号码输入，再弹值输入。
  final bool custom;
}

/// CC 常用名（仅展示用）。
const Map<int, String> _ccNames = {
  0: 'Bank Select MSB',
  1: 'Modulation',
  2: 'Breath',
  4: 'Foot',
  5: 'Portamento Time',
  6: 'Data Entry MSB',
  7: 'Volume',
  8: 'Balance',
  10: 'Pan',
  11: 'Expression',
  32: 'Bank Select LSB',
  38: 'Data Entry LSB',
  64: 'Sustain',
  65: 'Portamento',
  66: 'Sostenuto',
  67: 'Soft Pedal',
  71: 'Resonance',
  72: 'Release Time',
  73: 'Attack Time',
  74: 'Brightness',
  84: 'Portamento Control',
  91: 'Reverb Send',
  93: 'Chorus Send',
  120: 'All Sound Off',
  121: 'Reset All Controllers',
  123: 'All Notes Off',
};

const Map<int, String> _rpnNames = {
  0: 'Pitch Bend Sensitivity',
  1: 'Fine Tune',
  2: 'Coarse Tune',
};

/// 与自动化面板一致：`CC 11 Expression`（自定义号没有名字时只显示号）。
String ccInsertLabel(int cc) {
  final name = _ccNames[cc];
  return name == null ? 'CC $cc' : 'CC $cc $name';
}

/// `RPN 0 Pitch Bend Sensitivity`
String rpnInsertLabel(int rpn) {
  final name = _rpnNames[rpn];
  return name == null ? 'RPN $rpn' : 'RPN $rpn $name';
}

/// 每种自动化的图标（与语义对应）。
IconData _ccIcon(int cc) {
  switch (cc) {
    case 7:
      return Icons.volume_up;
    case 10:
      return Icons.balance;
    case 11:
      return Icons.graphic_eq;
    case 64:
      return Icons.piano;
    case 71:
      return Icons.tune;
    case 72:
      return Icons.timer_outlined;
    case 73:
      return Icons.bolt;
    case 74:
      return Icons.brightness_high;
    default:
      return Icons.tune;
  }
}

IconData _rpnIcon(int rpn) {
  switch (rpn) {
    case 0:
      return Icons.straighten;
    case 1:
      return Icons.tune;
    case 2:
      return Icons.tune;
    default:
      return Icons.tune;
  }
}

/// 显示值上限（与 Rust `AutomationTarget::display_max` 同规则）：
/// PB/RPN(1 及自定义) = 16383，CC/RPN 0/2 = 127，Tempo = 60000000 (BPM)。
int insertDisplayMax(int kind, int number) {
  switch (kind) {
    case 0:
      return 60000000;
    case 1:
      return 16383;
    case 2:
      return 127;
    case 3:
      return (number == 0 || number == 2) ? 127 : 16383;
  }
  return 127;
}

/// 无历史值时的默认显示值。
int insertDefaultValue(int kind, int number) {
  switch (kind) {
    case 0:
      return 120;
    case 1:
      return 8192; // 中心
    case 2:
      switch (number) {
        case 7:
          return 100;
        case 10:
        case 71:
        case 72:
        case 73:
        case 74:
          return 64;
        case 11:
          return 127;
        default:
          return 0;
      }
    case 3:
      switch (number) {
        case 0:
          return 2; // 半音
        case 1:
          return 8192; // 中心
        case 2:
          return 64; // 中心
        default:
          return 0;
      }
  }
  return 0;
}

/// 值输入标题（含中心提示）。
String insertValueTitle(int kind, int number) {
  switch (kind) {
    case 0:
      return '速度 BPM';
    case 1:
      return 'Pitch Bend（0–16383，8192 = 中心）';
    case 2:
      return 'CC $number ${ccInsertLabel(number)}（0–127）';
    case 3:
      return 'RPN $number ${rpnInsertLabel(number)}'
          '（${insertDisplayMax(kind, number) == 127 ? '0–127' : '0–16383'}）';
  }
  return '值';
}

/// 插入菜单：选择要插入的事件。返回 null = 取消。
Future<InsertChoice?> showInsertMenu(
  BuildContext context, {
  required bool conductor,
}) {
  final items = conductor
      ? <(IconData, InsertChoice)>[
          // Conductor 可插的全局事件：Tempo / 拍号 / 调号 / 标签。
          (Icons.speed, const InsertChoice(kind: 0, number: 0, label: 'Tempo')),
          (
            Icons.schedule,
            const InsertChoice(kind: 4, number: 0, label: '拍号 Time Signature'),
          ),
          (
            Icons.music_note,
            const InsertChoice(kind: 5, number: 0, label: '调号 Key Signature'),
          ),
          (
            Icons.bookmark_add_outlined,
            const InsertChoice(kind: 6, number: 0, label: '标签 Marker'),
          ),
        ]
      : <(IconData, InsertChoice)>[
          (
            Icons.swap_vert,
            const InsertChoice(kind: 1, number: 0, label: 'Pitch Bend'),
          ),
          for (final cc in const [7, 10, 11, 64, 71, 72, 73, 74])
            (
              _ccIcon(cc),
              InsertChoice(kind: 2, number: cc, label: ccInsertLabel(cc)),
            ),
          for (final rpn in const [0, 1, 2])
            (
              _rpnIcon(rpn),
              InsertChoice(kind: 3, number: rpn, label: rpnInsertLabel(rpn)),
            ),
          (
            Icons.tag,
            const InsertChoice(
              kind: 2,
              number: 0,
              label: '自定义 CC 号…',
              custom: true,
            ),
          ),
          (
            Icons.tag,
            const InsertChoice(
              kind: 3,
              number: 0,
              label: '自定义 RPN 号…',
              custom: true,
            ),
          ),
        ];
  return showModalBottomSheet<InsertChoice>(
    context: context,
    backgroundColor: YinheTheme.bg,
    isScrollControlled: true,
    builder: (ctx) => SafeArea(
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.sizeOf(ctx).height * 0.7,
        ),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final (icon, choice) in items)
                ListTile(
                  leading: Icon(icon),
                  title: Text(tr(choice.label)),
                  onTap: () => Navigator.of(ctx).pop(choice),
                ),
              ListTile(
                leading: const Icon(Icons.close),
                title: const Text('取消'),
                onTap: () => Navigator.of(ctx).pop(),
              ),
            ],
          ),
        ),
      ),
    ),
  );
}

/// 号码输入（自定义 CC / RPN）。
Future<int?> showInsertNumberDialog(
  BuildContext context, {
  required String title,
  required int min,
  required int max,
  required int initial,
}) {
  return showDialog<int>(
    context: context,
    builder: (_) =>
        _NumberDialog(title: title, min: min, max: max, initial: initial),
  );
}

/// 值输入；`quick` 提供快捷值（如 CC64 的 关闭/开启）。
Future<int?> showInsertValueDialog(
  BuildContext context, {
  required String title,
  required int min,
  required int max,
  required int initial,
  List<(String, int)> quick = const [],
}) {
  return showDialog<int>(
    context: context,
    builder: (_) => _NumberDialog(
      title: title,
      min: min,
      max: max,
      initial: initial,
      quick: quick,
    ),
  );
}

class _NumberDialog extends StatefulWidget {
  const _NumberDialog({
    required this.title,
    required this.min,
    required this.max,
    required this.initial,
    this.quick = const [],
  });

  final String title;
  final int min;
  final int max;
  final int initial;
  final List<(String, int)> quick;

  @override
  State<_NumberDialog> createState() => _NumberDialogState();
}

class _NumberDialogState extends State<_NumberDialog> {
  late final TextEditingController _controller = TextEditingController(
    text: '${widget.initial}',
  );
  String? _error;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _confirm() {
    final v = int.tryParse(_controller.text.trim());
    if (v == null || v < widget.min || v > widget.max) {
      setState(() => _error = '请输入 ${widget.min}–${widget.max} 之间的整数');
      return;
    }
    Navigator.of(context).pop(v);
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: Text(widget.title, style: const TextStyle(fontSize: 16)),
      content: SizedBox(
        width: 360,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const SizedBox(height: 10),
            TextField(
              controller: _controller,
              autofocus: true,
              keyboardType: TextInputType.number,
              style: const TextStyle(fontSize: 14),
              decoration: InputDecoration(
                labelText: '值（${widget.min}–${widget.max}）',
                border: const OutlineInputBorder(),
                isDense: true,
                errorText: _error,
              ),
              onSubmitted: (_) => _confirm(),
            ),
            if (widget.quick.isNotEmpty) ...[
              const SizedBox(height: 10),
              Wrap(
                spacing: 8,
                children: [
                  for (final (label, value) in widget.quick)
                    OutlinedButton(
                      onPressed: () =>
                          setState(() => _controller.text = '$value'),
                      child: Text('$label（$value）'),
                    ),
                ],
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('取消'),
        ),
        FilledButton(onPressed: _confirm, child: const Text('插入')),
      ],
    );
  }
}

/// 拍号输入（如 4/4）；返回 `(分子, 分母)`。
Future<(int, int)?> showInsertTimeSigDialog(
  BuildContext context, {
  int numerator = 4,
  int denominator = 4,
}) {
  return showDialog<(int, int)>(
    context: context,
    builder: (_) =>
        _TimeSigInputDialog(numerator: numerator, denominator: denominator),
  );
}

class _TimeSigInputDialog extends StatefulWidget {
  const _TimeSigInputDialog({
    required this.numerator,
    required this.denominator,
  });

  final int numerator;
  final int denominator;

  @override
  State<_TimeSigInputDialog> createState() => _TimeSigInputDialogState();
}

class _TimeSigInputDialogState extends State<_TimeSigInputDialog> {
  late final TextEditingController _sig = TextEditingController(
    text: '${widget.numerator}/${widget.denominator}',
  );
  String? _error;

  @override
  void dispose() {
    _sig.dispose();
    super.dispose();
  }

  void _confirm() {
    final parts = _sig.text.trim().split('/');
    final num = parts.length == 2 ? int.tryParse(parts[0].trim()) : null;
    final den = parts.length == 2 ? int.tryParse(parts[1].trim()) : null;
    if (num == null ||
        den == null ||
        num < 1 ||
        num > 32 ||
        den < 1 ||
        den > 32 ||
        (den & (den - 1)) != 0) {
      setState(() => _error = '格式如 4/4、3/4、6/8（分母需为 2 的幂）');
      return;
    }
    Navigator.of(context).pop((num, den));
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: const Text('拍号 Time Signature', style: TextStyle(fontSize: 16)),
      content: SizedBox(
        width: 340,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const SizedBox(height: 10),
            TextField(
              controller: _sig,
              autofocus: true,
              style: const TextStyle(fontSize: 14),
              decoration: InputDecoration(
                labelText: '拍号',
                border: const OutlineInputBorder(),
                isDense: true,
                errorText: _error,
              ),
              onSubmitted: (_) => _confirm(),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('取消'),
        ),
        FilledButton(onPressed: _confirm, child: const Text('插入')),
      ],
    );
  }
}

/// 调号输入；返回 `(主音 0=C..11=B, 0=大调 1=自然小调)`。
Future<(int, int)?> showInsertKeySigDialog(BuildContext context) {
  return showDialog<(int, int)>(
    context: context,
    builder: (_) => const _KeySigInputDialog(),
  );
}

class _KeySigInputDialog extends StatefulWidget {
  const _KeySigInputDialog();

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
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: const Text('调号 Key Signature', style: TextStyle(fontSize: 16)),
      content: SizedBox(
        width: 380,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const SizedBox(height: 10),
            DropdownButtonFormField<int>(
              initialValue: _root,
              decoration: const InputDecoration(
                labelText: '主音',
                border: OutlineInputBorder(),
                isDense: true,
              ),
              items: [
                for (var i = 0; i < _roots.length; i++)
                  DropdownMenuItem(value: i, child: Text(_roots[i])),
              ],
              onChanged: (v) => setState(() => _root = v ?? _root),
            ),
            const SizedBox(height: 12),
            DropdownButtonFormField<int>(
              initialValue: _scale,
              decoration: const InputDecoration(
                labelText: '调式',
                border: OutlineInputBorder(),
                isDense: true,
              ),
              items: const [
                DropdownMenuItem(value: 0, child: Text('大调 Major')),
                DropdownMenuItem(value: 1, child: Text('自然小调 Minor')),
              ],
              onChanged: (v) => setState(() => _scale = v ?? _scale),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop((_root, _scale)),
          child: const Text('插入'),
        ),
      ],
    );
  }
}

/// 单行文本输入（标签名等）。
Future<String?> showInsertTextDialog(
  BuildContext context, {
  required String title,
  String initial = '',
}) {
  return showDialog<String>(
    context: context,
    builder: (_) => _TextInputDialog(title: title, initial: initial),
  );
}

class _TextInputDialog extends StatefulWidget {
  const _TextInputDialog({required this.title, required this.initial});

  final String title;
  final String initial;

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
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: Text(widget.title, style: const TextStyle(fontSize: 16)),
      content: SizedBox(
        width: 340,
        child: Padding(
          padding: const EdgeInsets.only(top: 10),
          child: TextField(
            controller: _c,
            autofocus: true,
            style: const TextStyle(fontSize: 14),
            decoration: const InputDecoration(
              border: OutlineInputBorder(),
              isDense: true,
            ),
            onSubmitted: (v) => Navigator.of(context).pop(v),
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_c.text),
          child: const Text('确定'),
        ),
      ],
    );
  }
}
