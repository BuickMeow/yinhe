// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Japanese (`ja`).
class AppLocalizationsJa extends AppLocalizations {
  AppLocalizationsJa([String locale = 'ja']) : super(locale);

  @override
  String get cancel => 'キャンセル';

  @override
  String get save => '保存';

  @override
  String get ok => 'OK';

  @override
  String get back => '戻る';

  @override
  String get delete => '削除';

  @override
  String get edit => '編集';

  @override
  String get copy => 'Copy';

  @override
  String get paste => 'ペースト';

  @override
  String get close => '閉じる';

  @override
  String get add => '追加';

  @override
  String get select => '選択';

  @override
  String get clear => 'クリア';

  @override
  String get remove => '削除';

  @override
  String get insert => '挿入';

  @override
  String get value => '値';

  @override
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

  @override
  String notesCount(int count) {
    return '$count 音符';
  }

  @override
  String get openFile => 'ファイルを開く…';

  @override
  String get newProject => '新規プロジェクト';

  @override
  String get saveAs => '名前を付けて保存';

  @override
  String get settings => '設定';

  @override
  String get recentFiles => '最近変更したファイル';

  @override
  String get noRecentFiles => '最近のファイルはありません';

  @override
  String get backToWorkspace => 'ワークスペースに戻る';

  @override
  String get home => 'ホーム';

  @override
  String get backToArrangement => 'アレンジに戻る';

  @override
  String get play => '再生';

  @override
  String get pause => '一時停止';

  @override
  String get stop => '停止';

  @override
  String get quantize => 'クオンタイズ';

  @override
  String get quantizeOff => 'オフ（スナップなし）';

  @override
  String get quantizeAuto => '自動（ズーム連動）';

  @override
  String get tool => 'ツール';

  @override
  String get undo => '元に戻す';

  @override
  String get redo => 'やり直し';

  @override
  String get saveProject => '保存';

  @override
  String get saved => 'プロジェクトを保存しました';

  @override
  String get currentTrack => '編集中のトラック';

  @override
  String get allTracks => 'すべてを表示';

  @override
  String get project => 'プロジェクト';

  @override
  String get projectSettings => 'プロジェクト設定';

  @override
  String get songTitle => 'プロジェクト名';

  @override
  String get artist => 'アーティスト';

  @override
  String get description => '説明';

  @override
  String get ppq => '四分音符あたりのパルス数。ノートがある状態で変更するとリサイズを確認します';

  @override
  String get defaultBpm => 'プロジェクト既定 BPM';

  @override
  String get defaultTimeSig => 'プロジェクト既定拍子';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM must be 0–60000000';

  @override
  String get timeSigField => '拍子記号…';

  @override
  String get soundfonts => 'サウンドフォント';

  @override
  String get audio => 'オーディオ';

  @override
  String get language => '言語';

  @override
  String get followSystem => 'システムに従う';

  @override
  String get maxVoices => '最大同時発音数';

  @override
  String get audioBuffer => 'バッファサイズ';

  @override
  String get ignoreVelocity => 'ベロシティ無視';

  @override
  String get auto => '自動';

  @override
  String get deviceDefault => 'デバイス既定';

  @override
  String get addSoundfont => 'サウンドフォントを追加';

  @override
  String get frames => 'フレーム';

  @override
  String get editTrackInfo => '編集';

  @override
  String get eventList => 'イベント';

  @override
  String get deleteTrack => '削除';

  @override
  String get trackName => 'トラック名:';

  @override
  String get songTitleField => '曲タイトル:';

  @override
  String get port => 'ポート';

  @override
  String get channel => 'チャネル';

  @override
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => '選択';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => '編集';

  @override
  String get masterTrackHint =>
      'The master (Conductor) track holds tempo/time-signature events; it has no notes and no port/channel.';

  @override
  String get channelInstrumentHint =>
      'Channel default instrument is shared by all tracks on that channel; MIDI PC events take priority.';

  @override
  String get mobilePortHint =>
      'Only port A is audible on mobile; other ports are saved for the desktop app.';

  @override
  String get tempo => 'テンポ:';

  @override
  String get timeSignature => '拍子記号…';

  @override
  String get keySignature => '調号／スケール…';

  @override
  String get marker => 'マーカーを挿入';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'カスタムCC';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

  @override
  String get customRpn => 'カスタム RPN';

  @override
  String get root => 'ルート';

  @override
  String get scale => '調号';

  @override
  String get major => '長調 Major';

  @override
  String get minor => '自然短調 Minor';

  @override
  String get automationType => 'オートメーション…';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'パスワードが必要です';

  @override
  String get password => 'パスワードを入力';

  @override
  String get open => '開く';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'トラックを追加';

  @override
  String get trackCount => '数量';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'ペースト';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

  @override
  String get velocity => 'ベロシティ';

  @override
  String get length => 'ゲート';

  @override
  String get velocityRange => 'ベロシティ';

  @override
  String get lengthTicks => 'ゲート';

  @override
  String get editNote => 'ノート';

  @override
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'ノート';

  @override
  String get markerLabel => 'マーカーを挿入';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => '書き出しに失敗しました';

  @override
  String get copied => 'Copied';

  @override
  String get toolSelect => '選択';

  @override
  String get toolHand => '手のひら';

  @override
  String get toolPencil => '鉛筆';

  @override
  String get toolEraser => '消しゴム';

  @override
  String get toolBrush => 'ブラシ';
}
