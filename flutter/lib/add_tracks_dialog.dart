import 'package:flutter/material.dart';

import 'theme.dart';

/// 新建音轨对话框：数量 + 通道分配方式（相同通道 / 在所选 port 内循环）。
class AddTracksDialog extends StatefulWidget {
  const AddTracksDialog({
    super.key,
    required this.initialPort,
    required this.initialChannel,
    required this.onConfirm,
  });

  final int initialPort;
  final int initialChannel;
  final void Function(int count, int port, int channel, bool cycle) onConfirm;

  @override
  State<AddTracksDialog> createState() => _AddTracksDialogState();
}

class _AddTracksDialogState extends State<AddTracksDialog> {
  late final TextEditingController _count = TextEditingController(text: '1');
  late int _port = widget.initialPort.clamp(0, 15);
  late int _channel = widget.initialChannel.clamp(0, 15);

  /// true = 从通道开始循环（到 16 回到 1，不跨 port）；false = 全部相同通道。
  bool _cycle = true;
  String? _error;

  @override
  void dispose() {
    _count.dispose();
    super.dispose();
  }

  int get _parsedCount => int.tryParse(_count.text.trim()) ?? 0;

  String _channelCode(int channel) =>
      '${String.fromCharCode(0x41 + _port)}${channel + 1}';

  String _preview(int count) {
    final n = count.clamp(1, 16);
    final list = <String>[
      for (var i = 0; i < n; i++)
        _channelCode(_cycle ? (_channel + i) % 16 : _channel),
    ];
    final text = list.join('、');
    return count > n ? '$text …（共 $count 条）' : text;
  }

  void _save() {
    final count = _parsedCount;
    if (count < 1 || count > 64) {
      setState(() => _error = '数量需为 1–64');
      return;
    }
    widget.onConfirm(count, _port, _channel, _cycle);
    Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    final count = _parsedCount.clamp(1, 64);
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: const Text('添加音轨', style: TextStyle(fontSize: 17)),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              TextField(
                controller: _count,
                keyboardType: TextInputType.number,
                style: const TextStyle(fontSize: 14),
                decoration: InputDecoration(
                  labelText: '轨道数量（1–64）',
                  border: const OutlineInputBorder(),
                  isDense: true,
                  errorText: _error,
                ),
                onChanged: (_) => setState(() => _error = null),
              ),
              const SizedBox(height: 12),
              SegmentedButton<bool>(
                segments: const [
                  ButtonSegment(value: true, label: Text('从通道开始循环')),
                  ButtonSegment(value: false, label: Text('全部相同通道')),
                ],
                selected: {_cycle},
                onSelectionChanged: (s) => setState(() => _cycle = s.first),
              ),
              const SizedBox(height: 12),
              Row(
                children: [
                  Expanded(
                    child: DropdownButtonFormField<int>(
                      initialValue: _port,
                      decoration: const InputDecoration(
                        labelText: '端口',
                        border: OutlineInputBorder(),
                        isDense: true,
                      ),
                      items: [
                        for (var i = 0; i < 16; i++)
                          DropdownMenuItem(
                            value: i,
                            child: Text(String.fromCharCode(0x41 + i)),
                          ),
                      ],
                      onChanged: (v) => setState(() => _port = v ?? _port),
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: DropdownButtonFormField<int>(
                      initialValue: _channel,
                      decoration: const InputDecoration(
                        labelText: '起始通道',
                        border: OutlineInputBorder(),
                        isDense: true,
                      ),
                      items: [
                        for (var i = 0; i < 16; i++)
                          DropdownMenuItem(value: i, child: Text('${i + 1}')),
                      ],
                      onChanged: (v) =>
                          setState(() => _channel = v ?? _channel),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 10),
              Text(
                '将新建 $count 条：${_preview(count)}',
                style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
              ),
              const SizedBox(height: 4),
              Text(
                _cycle
                    ? '循环只在本端口内进行：到 16 后回到 1，不会跨到下一个端口。'
                    : '所有新音轨使用同一个端口和通道。',
                style: TextStyle(color: YinheTheme.textMuted, fontSize: 11),
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('取消'),
        ),
        FilledButton(onPressed: _save, child: const Text('添加')),
      ],
    );
  }
}
