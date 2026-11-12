import 'package:flutter/material.dart';

import '../core/midi_names.dart';
import '../core/models.dart';
import '../l10n/app_localizations.dart';
import '../core/theme.dart';
import 'frame.dart';

/// 通道级默认乐器：Bank MSB / Bank LSB / Program / 鼓通道。
///
/// 与桌面端同语义：三个分量对应 MIDI 的 Bank Select + Program Change，
/// 鼓通道是独立的通道模式开关（不占用 bank 号）。
class ChannelInstrumentValue {
  const ChannelInstrumentValue({
    this.bankMsb = 0,
    this.bankLsb = 0,
    this.program = 0,
    this.isDrum = false,
  });

  final int bankMsb;
  final int bankLsb;
  final int program;
  final bool isDrum;

  ChannelInstrumentValue copyWith({
    int? bankMsb,
    int? bankLsb,
    int? program,
    bool? isDrum,
  }) {
    return ChannelInstrumentValue(
      bankMsb: bankMsb ?? this.bankMsb,
      bankLsb: bankLsb ?? this.bankLsb,
      program: program ?? this.program,
      isDrum: isDrum ?? this.isDrum,
    );
  }

  /// Rust 侧 `channel_instruments_json` → `[bank_msb, bank_lsb, program, is_drum]`。
  static ChannelInstrumentValue? fromJson(dynamic raw) {
    if (raw is! List || raw.length < 3) {
      return null;
    }
    return ChannelInstrumentValue(
      bankMsb: (raw[0] as num).toInt(),
      bankLsb: (raw[1] as num).toInt(),
      program: (raw[2] as num).toInt(),
      // 旧后端只给两个分量；缺字段时按非鼓处理。
      isDrum: raw.length > 3 && (raw[3] as num).toInt() != 0,
    );
  }

  /// Program 的 GM 名字（鼓通道走鼓组表）。越界返回空串。
  String get programName =>
      MidiNames.instance.gmProgramLabel(isDrum ? 128 : 0, program);
}

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

  /// 16 个通道的默认乐器；null = 未设置（跟随 MIDI）。
  final List<ChannelInstrumentValue?> channelInstruments;

  /// `instrument == null` = 跟随 MIDI。
  final void Function(
    String name,
    int port,
    int channel,
    ChannelInstrumentValue? instrument,
  )
  onSave;

  @override
  State<TrackInfoDialog> createState() => _TrackInfoDialogState();
}

class _TrackInfoDialogState extends State<TrackInfoDialog> {
  AppLocalizations get l => AppLocalizations.of(context);

  late final TextEditingController _name = TextEditingController(
    text: widget.initialName,
  );
  late int _port = widget.initialPort.clamp(0, 15);
  late int _channel = widget.initialChannel.clamp(0, 15);
  late ChannelInstrumentValue? _instrument = _instrumentOf(_channel);

  ChannelInstrumentValue? _instrumentOf(int channel) {
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
    final picked = await showDialog<ChannelInstrumentValue?>(
      context: context,
      builder: (_) => _InstrumentPickerDialog(current: _instrument),
    );
    if (picked != null && mounted) {
      setState(() => _instrument = picked);
    }
  }

  void _save() {
    final name = _name.text.trim();
    widget.onSave(name, _port, _channel, _instrument);
    Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    final inst = _instrument;
    return YinheDialog(
      title: widget.isMaster ? l.masterTrack : l.editTrackInfo,
      width: 460,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(onPressed: _save, child: Text(l.save)),
      ],
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            controller: _name,
            style: const TextStyle(fontSize: 14),
            decoration: InputDecoration(
              labelText: widget.isMaster ? l.songTitleField : l.trackName,
              border: const OutlineInputBorder(),
              isDense: true,
            ),
          ),
          if (widget.isMaster) ...[
            const SizedBox(height: 12),
            Text(
              l.masterTrackHint,
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
                      labelText: l.port,
                      border: OutlineInputBorder(),
                      isDense: true,
                    ),
                    items: [
                      for (var i = 0; i < 16; i++)
                        DropdownMenuItem(value: i, child: Text(portLetter(i))),
                    ],
                    onChanged: (v) => setState(() => _port = v ?? _port),
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: DropdownButtonFormField<int>(
                    initialValue: _channel,
                    decoration: InputDecoration(
                      labelText: l.channel,
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
                        ? l.defaultInstrumentFollowPc
                        : '${inst.programName} · Bank ${inst.bankMsb}/'
                              '${inst.bankLsb} · Program ${inst.program}',
                    style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
                  ),
                ),
                TextButton(onPressed: _pickInstrument, child: Text(l.select)),
                if (inst != null)
                  TextButton(
                    onPressed: () => setState(() => _instrument = null),
                    child: Text(l.clear),
                  ),
              ],
            ),
            const SizedBox(height: 4),
            Text(
              l.channelInstrumentHint,
              style: TextStyle(color: YinheTheme.textMuted, fontSize: 11),
            ),
            if (_port != 0)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Text(
                  l.mobilePortHint,
                  style: TextStyle(color: YinheTheme.warning, fontSize: 11),
                ),
              ),
          ],
        ],
      ),
    );
  }
}

/// 乐器选择：Bank MSB / Bank LSB / Program 三个独立输入 + 鼓通道开关，
/// 下方列表按当前 Program 高亮（鼓通道时显示鼓组名）。
class _InstrumentPickerDialog extends StatefulWidget {
  const _InstrumentPickerDialog({required this.current});

  final ChannelInstrumentValue? current;

  @override
  State<_InstrumentPickerDialog> createState() =>
      _InstrumentPickerDialogState();
}

class _InstrumentPickerDialogState extends State<_InstrumentPickerDialog> {
  AppLocalizations get l => AppLocalizations.of(context);

  late ChannelInstrumentValue _value =
      widget.current ?? const ChannelInstrumentValue();

  @override
  Widget build(BuildContext context) {
    return YinheDialog(
      title: l.selectChannelInstrument,
      width: 420,
      scroll: false,
      content: SizedBox(
        height: 460,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Bank MSB / Bank LSB / Program：三个独立数字输入。
            Row(
              children: [
                Expanded(
                  child: _NumberField(
                    label: l.bankMsb,
                    value: _value.bankMsb,
                    onChanged: (v) =>
                        setState(() => _value = _value.copyWith(bankMsb: v)),
                  ),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: _NumberField(
                    label: l.bankLsb,
                    value: _value.bankLsb,
                    onChanged: (v) =>
                        setState(() => _value = _value.copyWith(bankLsb: v)),
                  ),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: _NumberField(
                    label: l.program,
                    value: _value.program,
                    onChanged: (v) =>
                        setState(() => _value = _value.copyWith(program: v)),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),
            SwitchListTile(
              dense: true,
              contentPadding: EdgeInsets.zero,
              title: Text(l.drumChannel, style: const TextStyle(fontSize: 13)),
              value: _value.isDrum,
              onChanged: (v) =>
                  setState(() => _value = _value.copyWith(isDrum: v)),
            ),
            const SizedBox(height: 4),
            Expanded(
              child: ListView.builder(
                itemCount: 128,
                itemBuilder: (context, i) {
                  final selected = i == _value.program;
                  final name = MidiNames.instance.gmProgramLabel(
                    _value.isDrum ? 128 : 0,
                    i,
                  );
                  return ListTile(
                    dense: true,
                    selected: selected,
                    title: Text(
                      '$i  $name',
                      style: TextStyle(
                        fontSize: 13,
                        color: selected ? YinheTheme.accent : YinheTheme.text,
                      ),
                    ),
                    onTap: () =>
                        setState(() => _value = _value.copyWith(program: i)),
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
          child: Text(l.cancel),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_value),
          child: Text(l.ok),
        ),
      ],
    );
  }
}

/// 0..127 的数字输入：带标签 + 校验，失焦/回车提交。
class _NumberField extends StatefulWidget {
  const _NumberField({
    required this.label,
    required this.value,
    required this.onChanged,
  });

  final String label;
  final int value;
  final ValueChanged<int> onChanged;

  @override
  State<_NumberField> createState() => _NumberFieldState();
}

class _NumberFieldState extends State<_NumberField> {
  late final TextEditingController _controller = TextEditingController(
    text: '${widget.value}',
  );

  @override
  void didUpdateWidget(_NumberField old) {
    super.didUpdateWidget(old);
    // 外部值变化（切换鼓通道等）时同步输入框，避免显示旧值。
    if (_controller.text != '${widget.value}') {
      _controller.text = '${widget.value}';
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _commit(String raw) {
    final parsed = int.tryParse(raw.trim());
    final clamped = (parsed ?? widget.value).clamp(0, 127);
    _controller.text = '$clamped';
    widget.onChanged(clamped);
  }

  @override
  Widget build(BuildContext context) {
    return TextField(
      controller: _controller,
      keyboardType: TextInputType.number,
      style: const TextStyle(fontSize: 13),
      decoration: InputDecoration(
        labelText: widget.label,
        border: const OutlineInputBorder(),
        isDense: true,
      ),
      onSubmitted: _commit,
      onTapOutside: (_) => _commit(_controller.text),
    );
  }
}
