import 'package:flutter/material.dart';

import 'theme.dart';
import 'l10n/app_localizations.dart';

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
  AppLocalizations get l => AppLocalizations.of(context);

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

  void _save() {
    final count = _parsedCount;
    if (count < 1 || count > 256) {
      setState(() => _error = l.countError);
      return;
    }
    widget.onConfirm(count, _port, _channel, _cycle);
    Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
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
              // 顶部留白：浮动 label 会超出输入框上缘，避免被裁掉一半。
              const SizedBox(height: 10),
              TextField(
                controller: _count,
                keyboardType: TextInputType.number,
                style: const TextStyle(fontSize: 14),
                decoration: InputDecoration(
                  labelText: l.trackCount,
                  border: const OutlineInputBorder(),
                  isDense: true,
                  errorText: _error,
                ),
                onChanged: (_) => setState(() => _error = null),
              ),
              const SizedBox(height: 12),
              SegmentedButton<bool>(
                segments: [
                  ButtonSegment(value: true, label: Text(l.cycleChannels)),
                  ButtonSegment(value: false, label: Text(l.sameChannel)),
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
                      decoration: InputDecoration(
                        labelText: l.port,
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
                      decoration: InputDecoration(
                        labelText: l.startChannel,
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
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(onPressed: _save, child: Text(l.add)),
      ],
    );
  }
}
