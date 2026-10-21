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
  String get allowOverlappingNotes => '音符の重なりを許可';

  @override
  String get copy => 'コピー';

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
  String get on => 'オン';

  @override
  String get off => 'オフ';

  @override
  String get loadMore => 'さらに読み込む';

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
  String get quantizeAuto => '自動';

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
  String get bpmField => 'BPM（0–60000000）';

  @override
  String get bpmError => 'BPM は 0–60000000 の範囲で指定してください';

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
  String get defaultInstrumentFollowPc => 'このチャンネルの既定音色：MIDI PC に従う';

  @override
  String get selectChannelInstrument => '選択';

  @override
  String get melodicBank => 'メロディ Bank 0';

  @override
  String get drumBank => 'ドラム Bank 128';

  @override
  String get masterTrack => '編集';

  @override
  String get masterTrackHint =>
      'マスター（Conductor）トラックはテンポ/拍子などのグローバルイベントを保持します。音符は置けず、ポート/チャンネル設定もありません。';

  @override
  String get channelInstrumentHint =>
      'チャンネルの既定音色：同じチャンネルの全トラックで共有されます。MIDI の PC イベントが優先されます。';

  @override
  String get mobilePortHint =>
      'モバイルではポート A のみ再生されます。他のポートは保存され、デスクトップ版で使用されます。';

  @override
  String get tempo => 'テンポ:';

  @override
  String get timeSignature => '拍子記号…';

  @override
  String get keySignature => '調号／スケール…';

  @override
  String get marker => 'マーカーを挿入';

  @override
  String get pitchBend => 'ピッチベンド';

  @override
  String get customCc => 'カスタムCC';

  @override
  String get customCcTitle => 'カスタム CC 番号';

  @override
  String get customRpnTitle => 'カスタム RPN 番号';

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
  String get tempoValueTitle => 'テンポ BPM';

  @override
  String get pitchBendValueTitle => 'ピッチベンド（0–16383、8192 = 中央）';

  @override
  String get archivePasswordRequired => 'パスワードが必要です';

  @override
  String get password => 'パスワードを入力';

  @override
  String get open => '開く';

  @override
  String get noMidiInArchive => '圧縮ファイルに MIDI ファイルがありません';

  @override
  String get archiveOpenFailed => '圧縮ファイルを開けませんでした';

  @override
  String get addTracks => 'トラックを追加';

  @override
  String get trackCount => '数量';

  @override
  String get countError => '数量は 1–256 で指定してください';

  @override
  String get cycleChannels => 'チャンネルから順に割り当て';

  @override
  String get sameChannel => 'すべて同じチャンネル';

  @override
  String get startChannel => '開始チャンネル';

  @override
  String get pasteSameTime => 'ペースト';

  @override
  String get goToLocation => '位置へ移動';

  @override
  String get eventCopied => 'イベントをコピーしました';

  @override
  String get noEvents => 'イベントなし';

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
  String get sortHint => '時間 → 音高/値 → 長さ';

  @override
  String get noteLabel => 'ノート';

  @override
  String get markerLabel => 'マーカーを挿入';

  @override
  String get openFirst => '先に MIDI を開くか、プロジェクトを作成してください';

  @override
  String get exportFailed => '書き出しに失敗しました';

  @override
  String get copied => 'コピーしました';

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

  @override
  String get markerRename => 'ラベルの名前を変更';

  @override
  String get markerDelete => 'ラベルを削除';

  @override
  String get markerText => 'ラベルのテキスト';

  @override
  String get markerTextOptional => 'ラベルのテキスト（空欄可）';

  @override
  String get untitled => '無題';

  @override
  String get ppqError => 'PPQ は 1–32767 の整数で指定してください';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return 'このチャンネルの既定音色：$name（Bank $bank / Program $program）';
  }

  @override
  String eventNoteTitle(String name) {
    return '音符 $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · ベロシティ $velocity · 長さ $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value（0–$max）';
  }

  @override
  String get centerHint => '8192 = 中央';

  @override
  String get jumpTo => 'ジャンプ';

  @override
  String get exportAudio => '音声を書き出す';

  @override
  String get exportMidi => 'MIDI を書き出す';

  @override
  String get exporting => '書き出し中…';

  @override
  String get exportDone => '書き出し完了';

  @override
  String get bitDepth => 'ビット深度';

  @override
  String get cancelling => 'キャンセル中…';

  @override
  String get exportFailedShort => '書き出し失敗';

  @override
  String get prevPage => '前のページ';

  @override
  String get nextPage => '次のページ';

  @override
  String get importingSoundfont => '音色ライブラリを読み込み中…';

  @override
  String get importSoundfontDone => '音色ライブラリを読み込みました';

  @override
  String get noSoundfontInArchive => '圧縮ファイルに音色ライブラリがありません';

  @override
  String get loadingFile => '読み込み中…';

  @override
  String get loadFailed => '読み込みに失敗しました';

  @override
  String get savingProject => 'プロジェクトを保存中…';

  @override
  String get saveFailed => '保存に失敗しました';

  @override
  String get unsavedChanges => '未保存の変更があります';

  @override
  String get unsavedChangesPrompt => '続行すると現在の変更は破棄されます。先に保存しますか？';

  @override
  String get dontSave => '保存しない';
}
