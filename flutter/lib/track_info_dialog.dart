import 'package:flutter/material.dart';

import 'gm_programs.dart';
import 'l10n.dart';
import 'theme.dart';

/// 音轨信息对话框：名称 / 端口 / 通道 / 通道级默认乐器。
///
/// Master（Conductor）轨只允许改名称（= 歌曲标题），其余字段忽略。
class TrackInfoDialog extends StatefulWidget {
  const TrackInfoDialog({
    super.key,
    required this.initialName,
    required this.initialPort,
    required this.initialChannel,
    required this.isMaster,
    required this.channelInstruments,
    required this.onSave,
  });

  final String initialName;
  final int initialPort;
  final int initialChannel;
  final bool isMaster;

  /// 16 个通道的默认乐器 `(bank, program)`；null = 未设置。
  final List<(int, int)?> channelInstruments;
  final void Function(
    String name,
    int port,
    int channel,
    int? bank,
    int? program,
  )
  onSave;

  @override
  State<TrackInfoDialog> createState() => _TrackInfoDialogState();
}

class _TrackInfoDialogState extends State<TrackInfoDialog> {
  late final TextEditingController _name = TextEditingController(
    text: widget.initialName,
  );
  late int _port = widget.initialPort.clamp(0, 15);
  late int _channel = widget.initialChannel.clamp(0, 15);
  late (int, int)? _instrument = _instrumentOf(_channel);

  (int, int)? _instrumentOf(int channel) {
    if (channel < 0 || channel >= widget.channelInstruments.length) {
      return null;
    }
    return widget.channelInstruments[channel];
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _pickInstrument() async {
    final picked = await showDialog<(int, int)?>(
      context: context,
      builder: (_) => _InstrumentPickerDialog(current: _instrument),
    );
    if (picked != null && mounted) {
      setState(() => _instrument = picked);
    }
  }

  void _save() {
    final name = _name.text.trim();
    widget.onSave(name, _port, _channel, _instrument?.$1, _instrument?.$2);
    Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    final inst = _instrument;
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: Text(
        widget.isMaster ? '编辑 Master 轨信息' : '编辑音轨信息',
        style: const TextStyle(fontSize: 17),
      ),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const SizedBox(height: 10),
              TextField(
                controller: _name,
                style: const TextStyle(fontSize: 14),
                decoration: InputDecoration(
                  labelText: tr(widget.isMaster ? '歌曲标题' : '音轨名称'),
                  border: const OutlineInputBorder(),
                  isDense: true,
                ),
              ),
              if (widget.isMaster) ...[
                const SizedBox(height: 12),
                Text(
                  tr('Master（Conductor）轨承载速度/拍号等全局事件，不能放置音符，也没有端口/通道设置。'),
                  style: TextStyle(color: YinheTheme.textMuted, fontSize: 12),
                ),
              ] else ...[
                const SizedBox(height: 12),
                Row(
                  children: [
                    Expanded(
                      child: DropdownButtonFormField<int>(
                        initialValue: _port,
                        decoration: InputDecoration(
                          labelText: tr('端口'),
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
                          labelText: tr('通道'),
                          border: OutlineInputBorder(),
                          isDense: true,
                        ),
                        items: [
                          for (var i = 0; i < 16; i++)
                            DropdownMenuItem(value: i, child: Text('${i + 1}')),
                        ],
                        onChanged: (v) {
                          if (v == null) {
                            return;
                          }
                          // 通道级设置：切换通道时显示目标通道当前的默认乐器。
                          setState(() {
                            _channel = v;
                            _instrument = _instrumentOf(v);
                          });
                        },
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 12),
                Row(
                  children: [
                    Expanded(
                      child: Text(
                        inst == null
                            ? tr('本通道默认乐器：跟随 MIDI PC')
                            : '本通道默认乐器：${gmProgramLabel(inst.$1, inst.$2)}'
                                  '（Bank ${inst.$1} / Program ${inst.$2}）',
                        style: TextStyle(
                          color: YinheTheme.textLabel,
                          fontSize: 13,
                        ),
                      ),
                    ),
                    TextButton(
                      onPressed: _pickInstrument,
                      child: Text(tr('选择')),
                    ),
                    if (inst != null)
                      TextButton(
                        onPressed: () => setState(() => _instrument = null),
                        child: Text(tr('清除')),
                      ),
                  ],
                ),
                const SizedBox(height: 4),
                Text(
                  tr('通道级默认乐器：同一通道的所有音轨共享；MIDI 内的 PC 事件优先于它。'),
                  style: TextStyle(color: YinheTheme.textMuted, fontSize: 11),
                ),
                if (_port != 0)
                  Padding(
                    padding: const EdgeInsets.only(top: 4),
                    child: Text(
                      tr('移动端目前仅端口 A 参与播放；其他端口会保存，供桌面端使用。'),
                      style: TextStyle(color: YinheTheme.warning, fontSize: 11),
                    ),
                  ),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(tr('取消')),
        ),
        FilledButton(onPressed: _save, child: Text(tr('保存'))),
      ],
    );
  }
}

/// 乐器选择：先选 Bank（旋律/鼓组），再选 Program。
class _InstrumentPickerDialog extends StatefulWidget {
  const _InstrumentPickerDialog({required this.current});

  final (int, int)? current;

  @override
  State<_InstrumentPickerDialog> createState() =>
      _InstrumentPickerDialogState();
}

class _InstrumentPickerDialogState extends State<_InstrumentPickerDialog> {
  late int _bank = widget.current?.$1 == 128 ? 128 : 0;
  late int _program = widget.current?.$2.clamp(0, 127) ?? 0;

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: Text(tr('选择本通道默认乐器'), style: TextStyle(fontSize: 17)),
      content: SizedBox(
        width: 420,
        height: 460,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SegmentedButton<int>(
              segments: [
                ButtonSegment(value: 0, label: Text(tr('旋律 Bank 0'))),
                ButtonSegment(value: 128, label: Text(tr('鼓组 Bank 128'))),
              ],
              selected: {_bank},
              onSelectionChanged: (s) => setState(() => _bank = s.first),
            ),
            const SizedBox(height: 8),
            Expanded(
              child: ListView.builder(
                itemCount: 128,
                itemBuilder: (context, i) {
                  final selected = i == _program;
                  return ListTile(
                    dense: true,
                    selected: selected,
                    title: Text(
                      '$i  ${gmProgramLabel(_bank, i)}',
                      style: TextStyle(
                        fontSize: 13,
                        color: selected ? YinheTheme.accent : YinheTheme.text,
                      ),
                    ),
                    onTap: () => setState(() => _program = i),
                  );
                },
              ),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(tr('取消')),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop((_bank, _program)),
          child: Text(tr('确定')),
        ),
      ],
    );
  }
}
