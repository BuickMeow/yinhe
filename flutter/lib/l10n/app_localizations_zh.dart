// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Chinese (`zh`).
class AppLocalizationsZh extends AppLocalizations {
  AppLocalizationsZh([String locale = 'zh']) : super(locale);

  @override
  String get cancel => '取消';

  @override
  String get save => '保存';

  @override
  String get ok => '确定';

  @override
  String get back => '返回';

  @override
  String get delete => '删除';

  @override
  String get edit => '修改';

  @override
  String get copy => '复制';

  @override
  String get paste => '粘贴';

  @override
  String get close => '关闭';

  @override
  String get add => '添加';

  @override
  String get select => '选择';

  @override
  String get clear => '清除';

  @override
  String get remove => '移除';

  @override
  String get insert => '插入';

  @override
  String get value => '值';

  @override
  String get on => '开启';

  @override
  String get off => '关闭';

  @override
  String get loadMore => '加载更多';

  @override
  String notesCount(int count) {
    return '$count 音符';
  }

  @override
  String get openFile => '打开文件';

  @override
  String get newProject => '新建工程';

  @override
  String get saveAs => '另存为';

  @override
  String get settings => '设置';

  @override
  String get recentFiles => '最近修改的文件';

  @override
  String get noRecentFiles => '暂无最近文件';

  @override
  String get backToWorkspace => '返回工作区';

  @override
  String get home => '主页';

  @override
  String get backToArrangement => '返回走带';

  @override
  String get play => '播放';

  @override
  String get pause => '暂停';

  @override
  String get stop => '停止';

  @override
  String get quantize => '量化（吸附档位）';

  @override
  String get quantizeOff => '关闭吸附';

  @override
  String get quantizeAuto => '自动';

  @override
  String get tool => '工具';

  @override
  String get undo => '撤销';

  @override
  String get redo => '重做';

  @override
  String get saveProject => '保存工程';

  @override
  String get saved => '已保存';

  @override
  String get currentTrack => '当前编辑音轨';

  @override
  String get allTracks => '全部轨道';

  @override
  String get project => '工程';

  @override
  String get projectSettings => '工程设置';

  @override
  String get songTitle => '歌名';

  @override
  String get artist => '艺术家';

  @override
  String get description => '简介';

  @override
  String get ppq => 'PPQ（已加载音符时修改会缩放全部时间）';

  @override
  String get defaultBpm => '本工程默认 BPM';

  @override
  String get defaultTimeSig => '本工程默认拍号';

  @override
  String get bpmField => 'BPM（0–60000000）';

  @override
  String get bpmError => 'BPM 需为 0–60000000';

  @override
  String get timeSigField => '拍号（如 4/4）';

  @override
  String get soundfonts => '音色库';

  @override
  String get audio => '音频';

  @override
  String get language => '语言';

  @override
  String get followSystem => '跟随系统';

  @override
  String get maxVoices => '复音上限';

  @override
  String get audioBuffer => '音频缓冲';

  @override
  String get ignoreVelocity => '忽略力度';

  @override
  String get auto => '自动';

  @override
  String get deviceDefault => '设备默认';

  @override
  String get addSoundfont => '添加音色库';

  @override
  String get frames => '帧';

  @override
  String get editTrackInfo => '编辑音轨信息';

  @override
  String get eventList => '事件列表';

  @override
  String get deleteTrack => '删除音轨';

  @override
  String get trackName => '音轨名称';

  @override
  String get songTitleField => '歌曲标题';

  @override
  String get port => '端口';

  @override
  String get channel => '通道';

  @override
  String get defaultInstrumentFollowPc => '本通道默认乐器：跟随 MIDI PC';

  @override
  String get selectChannelInstrument => '选择本通道默认乐器';

  @override
  String get melodicBank => '旋律 Bank 0';

  @override
  String get drumBank => '鼓组 Bank 128';

  @override
  String get masterTrack => '编辑 Master 轨信息';

  @override
  String get masterTrackHint =>
      'Master（Conductor）轨承载速度/拍号等全局事件，不能放置音符，也没有端口/通道设置。';

  @override
  String get channelInstrumentHint => '通道级默认乐器：同一通道的所有音轨共享；MIDI 内的 PC 事件优先于它。';

  @override
  String get mobilePortHint => '移动端目前仅端口 A 参与播放；其他端口会保存，供桌面端使用。';

  @override
  String get tempo => '速度（Tempo）';

  @override
  String get timeSignature => '拍号 Time Signature';

  @override
  String get keySignature => '调号 Key Signature';

  @override
  String get marker => '标签 Marker';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => '自定义 CC 号…';

  @override
  String get customCcTitle => '自定义 CC 号';

  @override
  String get customRpnTitle => '自定义 RPN 号';

  @override
  String get customRpn => '自定义 RPN 号…';

  @override
  String get root => '主音';

  @override
  String get scale => '调式';

  @override
  String get major => '大调 Major';

  @override
  String get minor => '自然小调 Minor';

  @override
  String get automationType => '自动化类型';

  @override
  String get tempoValueTitle => '速度 BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend（0–16383，8192 = 中心）';

  @override
  String get archivePasswordRequired => '压缩包需要密码';

  @override
  String get password => '密码';

  @override
  String get open => '打开';

  @override
  String get noMidiInArchive => '压缩包里没有 MIDI 文件';

  @override
  String get archiveOpenFailed => '压缩包打开失败';

  @override
  String get addTracks => '添加音轨';

  @override
  String get trackCount => '轨道数量（1–256）';

  @override
  String get countError => '数量需为 1–256';

  @override
  String get cycleChannels => '从通道开始循环';

  @override
  String get sameChannel => '全部相同通道';

  @override
  String get startChannel => '起始通道';

  @override
  String get pasteSameTime => '粘贴（同时间位置）';

  @override
  String get goToLocation => '前往该地点';

  @override
  String get eventCopied => '已复制事件';

  @override
  String get noEvents => '没有事件';

  @override
  String get velocity => '力度';

  @override
  String get length => '长度';

  @override
  String get velocityRange => '力度（1–127）';

  @override
  String get lengthTicks => '长度（tick）';

  @override
  String get editNote => '修改音符';

  @override
  String get sortHint => '时间 → 音高/值 → 时长';

  @override
  String get noteLabel => '音符';

  @override
  String get markerLabel => '标签';

  @override
  String get openFirst => '请先打开 MIDI 或新建工程';

  @override
  String get exportFailed => '工程导出失败';

  @override
  String get copied => '已复制';

  @override
  String get toolSelect => '选择';

  @override
  String get toolHand => '手形';

  @override
  String get toolPencil => '铅笔';

  @override
  String get toolEraser => '橡皮擦';

  @override
  String get toolBrush => '刷子';

  @override
  String get markerRename => '重命名标签';

  @override
  String get markerDelete => '删除标签';

  @override
  String get markerText => '标签文本';

  @override
  String get markerTextOptional => '标签文本（可留空）';

  @override
  String get untitled => '未命名';

  @override
  String get ppqError => 'PPQ 需为 1–32767 的整数';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return '本通道默认乐器：$name（Bank $bank / Program $program）';
  }

  @override
  String eventNoteTitle(String name) {
    return '音符 $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · 力度 $velocity · 长度 $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value（0–$max）';
  }

  @override
  String get centerHint => '8192 = 中心';

  @override
  String get jumpTo => '跳转';

  @override
  String get exportAudio => '导出音频';

  @override
  String get exportMidi => '导出 MIDI';

  @override
  String get exporting => '正在导出…';

  @override
  String get exportDone => '导出完成';

  @override
  String get bitDepth => '位深';

  @override
  String get cancelling => '正在取消…';

  @override
  String get exportFailedShort => '导出失败';

  @override
  String get prevPage => '上一页';

  @override
  String get nextPage => '下一页';

  @override
  String get importingSoundfont => '正在导入音色库…';

  @override
  String get importSoundfontDone => '音色库导入完成';

  @override
  String get noSoundfontInArchive => '压缩包里没有音色库';

  @override
  String get loadingFile => '正在加载…';

  @override
  String get loadFailed => '加载失败';

  @override
  String get savingProject => '正在保存工程…';

  @override
  String get saveFailed => '保存失败';
}

/// The translations for Chinese, as used in Hong Kong (`zh_HK`).
class AppLocalizationsZhHk extends AppLocalizationsZh {
  AppLocalizationsZhHk() : super('zh_HK');

  @override
  String get cancel => '取消';

  @override
  String get save => '儲存';

  @override
  String get ok => '確定';

  @override
  String get back => '返回';

  @override
  String get delete => '刪除';

  @override
  String get edit => '編輯';

  @override
  String get copy => '複製';

  @override
  String get paste => '貼上';

  @override
  String get close => '關閉';

  @override
  String get add => '新增';

  @override
  String get select => '選取';

  @override
  String get clear => '清除';

  @override
  String get remove => '移除';

  @override
  String get insert => '插入';

  @override
  String get value => '數值';

  @override
  String get on => '開啟';

  @override
  String get off => '關閉';

  @override
  String get loadMore => '載入更多';

  @override
  String notesCount(int count) {
    return '$count 音符';
  }

  @override
  String get openFile => '開啟檔案…';

  @override
  String get newProject => '新增工程';

  @override
  String get saveAs => '另存新檔';

  @override
  String get settings => '設定';

  @override
  String get recentFiles => '最近修改的檔案';

  @override
  String get noRecentFiles => '暫無最近檔案';

  @override
  String get backToWorkspace => '返回工作區';

  @override
  String get home => '主頁';

  @override
  String get backToArrangement => '返回走帶';

  @override
  String get play => '播放';

  @override
  String get pause => '暫停';

  @override
  String get stop => '停止';

  @override
  String get quantize => '量化（吸附檔位）';

  @override
  String get quantizeOff => '關閉吸附';

  @override
  String get quantizeAuto => '自動';

  @override
  String get tool => '工具';

  @override
  String get undo => '還原';

  @override
  String get redo => '重做';

  @override
  String get saveProject => '儲存';

  @override
  String get saved => '工程已儲存';

  @override
  String get currentTrack => '目前編輯音軌';

  @override
  String get allTracks => '全部顯示';

  @override
  String get project => '工程';

  @override
  String get projectSettings => '工程設定';

  @override
  String get songTitle => '項目名稱';

  @override
  String get artist => '藝術家';

  @override
  String get description => '簡介';

  @override
  String get ppq => '每四分音符的脈衝數，修改含音符工程時會提示縮放';

  @override
  String get defaultBpm => '本工程預設 BPM';

  @override
  String get defaultTimeSig => '本工程預設拍號';

  @override
  String get bpmField => 'BPM（0–60000000）';

  @override
  String get bpmError => 'BPM 需為 0–60000000';

  @override
  String get timeSigField => '拍號…';

  @override
  String get soundfonts => '音色庫';

  @override
  String get audio => '音訊';

  @override
  String get language => '語言';

  @override
  String get followSystem => '跟隨系統';

  @override
  String get maxVoices => '最大複音數';

  @override
  String get audioBuffer => '緩衝區大小';

  @override
  String get ignoreVelocity => '忽略力度';

  @override
  String get auto => '自動';

  @override
  String get deviceDefault => '裝置預設';

  @override
  String get addSoundfont => '新增音色庫';

  @override
  String get frames => '幀';

  @override
  String get editTrackInfo => '編輯';

  @override
  String get eventList => '事件';

  @override
  String get deleteTrack => '刪除';

  @override
  String get trackName => '音軌名稱:';

  @override
  String get songTitleField => '歌曲標題:';

  @override
  String get port => 'Port';

  @override
  String get channel => 'Channel';

  @override
  String get defaultInstrumentFollowPc => '本通道預設樂器：跟隨 MIDI PC';

  @override
  String get selectChannelInstrument => '選取';

  @override
  String get melodicBank => '旋律 Bank 0';

  @override
  String get drumBank => '鼓組 Bank 128';

  @override
  String get masterTrack => '編輯';

  @override
  String get masterTrackHint =>
      'Master（Conductor）軌承載速度/拍號等全域事件，不能放置音符，也沒有連接埠/通道設定。';

  @override
  String get channelInstrumentHint => '通道級預設樂器：同一通道的所有音軌共用；MIDI 內的 PC 事件優先於它。';

  @override
  String get mobilePortHint => '行動端目前僅連接埠 A 參與播放；其他連接埠會儲存，供桌面端使用。';

  @override
  String get tempo => 'Tempo:';

  @override
  String get timeSignature => '拍號…';

  @override
  String get keySignature => '調號／調式…';

  @override
  String get marker => '插入標籤';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => '自訂 CC';

  @override
  String get customCcTitle => '自訂 CC 號';

  @override
  String get customRpnTitle => '自訂 RPN 號';

  @override
  String get customRpn => '自訂 RPN';

  @override
  String get root => '主音';

  @override
  String get scale => '調式';

  @override
  String get major => '大調 Major';

  @override
  String get minor => '自然小調 Minor';

  @override
  String get automationType => '自動化…';

  @override
  String get tempoValueTitle => '速度 BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend（0–16383，8192 = 中心）';

  @override
  String get archivePasswordRequired => '壓縮檔需要密碼';

  @override
  String get password => '請輸入密碼';

  @override
  String get open => '開啟';

  @override
  String get noMidiInArchive => '壓縮檔裡沒有 MIDI 檔案';

  @override
  String get archiveOpenFailed => '壓縮檔開啟失敗';

  @override
  String get addTracks => '新增音軌';

  @override
  String get trackCount => '數量';

  @override
  String get countError => '數量需為 1–256';

  @override
  String get cycleChannels => '從通道開始循環';

  @override
  String get sameChannel => '全部相同通道';

  @override
  String get startChannel => '起始通道';

  @override
  String get pasteSameTime => '貼上';

  @override
  String get goToLocation => '前往該地點';

  @override
  String get eventCopied => '已複製事件';

  @override
  String get noEvents => '沒有事件';

  @override
  String get velocity => '力度';

  @override
  String get length => '長度';

  @override
  String get velocityRange => '力度';

  @override
  String get lengthTicks => '長度';

  @override
  String get editNote => '音符';

  @override
  String get sortHint => '時間 → 音高/值 → 長度';

  @override
  String get noteLabel => '音符';

  @override
  String get markerLabel => '插入標籤';

  @override
  String get openFirst => '請先開啟 MIDI 或新建專案';

  @override
  String get exportFailed => '匯出失敗';

  @override
  String get copied => '已複製';

  @override
  String get toolSelect => '選取';

  @override
  String get toolHand => '手形';

  @override
  String get toolPencil => '鉛筆';

  @override
  String get toolEraser => '橡皮擦';

  @override
  String get toolBrush => '刷子';

  @override
  String get markerRename => '重新命名標籤';

  @override
  String get markerDelete => '刪除標籤';

  @override
  String get markerText => '標籤文字';

  @override
  String get markerTextOptional => '標籤文字（可留空）';

  @override
  String get untitled => '未命名';

  @override
  String get ppqError => 'PPQ 需為 1–32767 的整數';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return '本通道預設樂器：$name（Bank $bank / Program $program）';
  }

  @override
  String eventNoteTitle(String name) {
    return '音符 $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · 力度 $velocity · 長度 $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value（0–$max）';
  }

  @override
  String get centerHint => '8192 = 中心';

  @override
  String get jumpTo => '跳轉';

  @override
  String get exportAudio => '匯出音訊';

  @override
  String get exportMidi => '匯出 MIDI';

  @override
  String get exporting => '正在匯出…';

  @override
  String get exportDone => '匯出完成';

  @override
  String get bitDepth => '位元深度';

  @override
  String get cancelling => '正在取消…';

  @override
  String get exportFailedShort => '匯出失敗';

  @override
  String get prevPage => '上一頁';

  @override
  String get nextPage => '下一頁';

  @override
  String get importingSoundfont => '正在匯入音色庫…';

  @override
  String get importSoundfontDone => '音色庫匯入完成';

  @override
  String get noSoundfontInArchive => '壓縮檔裡沒有音色庫';

  @override
  String get loadingFile => '正在載入…';

  @override
  String get loadFailed => '載入失敗';

  @override
  String get savingProject => '正在儲存工程…';

  @override
  String get saveFailed => '儲存失敗';
}

/// The translations for Chinese, as used in Taiwan (`zh_TW`).
class AppLocalizationsZhTw extends AppLocalizationsZh {
  AppLocalizationsZhTw() : super('zh_TW');

  @override
  String get cancel => '取消';

  @override
  String get save => '儲存';

  @override
  String get ok => '確定';

  @override
  String get back => '返回';

  @override
  String get delete => '刪除';

  @override
  String get edit => '編輯';

  @override
  String get copy => '複製';

  @override
  String get paste => '貼上';

  @override
  String get close => '關閉';

  @override
  String get add => '新增';

  @override
  String get select => '選取';

  @override
  String get clear => '清除';

  @override
  String get remove => '移除';

  @override
  String get insert => '插入';

  @override
  String get value => '數值';

  @override
  String get on => '開啟';

  @override
  String get off => '關閉';

  @override
  String get loadMore => '載入更多';

  @override
  String notesCount(int count) {
    return '$count 音符';
  }

  @override
  String get openFile => '開啟檔案…';

  @override
  String get newProject => '新增工程';

  @override
  String get saveAs => '另存新檔';

  @override
  String get settings => '設定';

  @override
  String get recentFiles => '最近修改的檔案';

  @override
  String get noRecentFiles => '暫無最近檔案';

  @override
  String get backToWorkspace => '返回工作區';

  @override
  String get home => '主頁';

  @override
  String get backToArrangement => '返回走帶';

  @override
  String get play => '播放';

  @override
  String get pause => '暫停';

  @override
  String get stop => '停止';

  @override
  String get quantize => '量化（吸附檔位）';

  @override
  String get quantizeOff => '關閉吸附';

  @override
  String get quantizeAuto => '自動';

  @override
  String get tool => '工具';

  @override
  String get undo => '復原';

  @override
  String get redo => '重做';

  @override
  String get saveProject => '儲存';

  @override
  String get saved => '工程已儲存';

  @override
  String get currentTrack => '目前編輯音軌';

  @override
  String get allTracks => '全部顯示';

  @override
  String get project => '工程';

  @override
  String get projectSettings => '工程設定';

  @override
  String get songTitle => '專案名稱';

  @override
  String get artist => '藝術家';

  @override
  String get description => '簡介';

  @override
  String get ppq => '每四分音符的脈衝數，修改含音符專案時會提示縮放';

  @override
  String get defaultBpm => '本工程預設 BPM';

  @override
  String get defaultTimeSig => '本工程預設拍號';

  @override
  String get bpmField => 'BPM（0–60000000）';

  @override
  String get bpmError => 'BPM 需為 0–60000000';

  @override
  String get timeSigField => '拍號…';

  @override
  String get soundfonts => '音色庫';

  @override
  String get audio => '音訊';

  @override
  String get language => '語言';

  @override
  String get followSystem => '跟隨系統';

  @override
  String get maxVoices => '最大複音數';

  @override
  String get audioBuffer => '緩衝區大小';

  @override
  String get ignoreVelocity => '忽略力度';

  @override
  String get auto => '自動';

  @override
  String get deviceDefault => '裝置預設';

  @override
  String get addSoundfont => '新增音色庫';

  @override
  String get frames => '幀';

  @override
  String get editTrackInfo => '編輯';

  @override
  String get eventList => '事件';

  @override
  String get deleteTrack => '刪除';

  @override
  String get trackName => '音軌名稱:';

  @override
  String get songTitleField => '歌曲標題:';

  @override
  String get port => 'Port';

  @override
  String get channel => '通道';

  @override
  String get defaultInstrumentFollowPc => '本通道預設樂器：跟隨 MIDI PC';

  @override
  String get selectChannelInstrument => '選取';

  @override
  String get melodicBank => '旋律 Bank 0';

  @override
  String get drumBank => '鼓組 Bank 128';

  @override
  String get masterTrack => '編輯';

  @override
  String get masterTrackHint =>
      'Master（Conductor）軌承載速度/拍號等全域事件，不能放置音符，也沒有連接埠/通道設定。';

  @override
  String get channelInstrumentHint => '通道級預設樂器：同一通道的所有音軌共用；MIDI 內的 PC 事件優先於它。';

  @override
  String get mobilePortHint => '行動端目前僅連接埠 A 參與播放；其他連接埠會儲存，供桌面端使用。';

  @override
  String get tempo => 'Tempo:';

  @override
  String get timeSignature => '拍號…';

  @override
  String get keySignature => '調號／調式…';

  @override
  String get marker => '插入標籤';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => '自訂 CC';

  @override
  String get customCcTitle => '自訂 CC 號';

  @override
  String get customRpnTitle => '自訂 RPN 號';

  @override
  String get customRpn => '自訂 RPN';

  @override
  String get root => '主音';

  @override
  String get scale => '調式';

  @override
  String get major => '大調 Major';

  @override
  String get minor => '自然小調 Minor';

  @override
  String get automationType => '自動化…';

  @override
  String get tempoValueTitle => '速度 BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend（0–16383，8192 = 中心）';

  @override
  String get archivePasswordRequired => '壓縮檔需要密碼';

  @override
  String get password => '請輸入密碼';

  @override
  String get open => '開啟';

  @override
  String get noMidiInArchive => '壓縮檔裡沒有 MIDI 檔案';

  @override
  String get archiveOpenFailed => '壓縮檔開啟失敗';

  @override
  String get addTracks => '新增音軌';

  @override
  String get trackCount => '數量';

  @override
  String get countError => '數量需為 1–256';

  @override
  String get cycleChannels => '從通道開始循環';

  @override
  String get sameChannel => '全部相同通道';

  @override
  String get startChannel => '起始通道';

  @override
  String get pasteSameTime => '貼上';

  @override
  String get goToLocation => '前往該地點';

  @override
  String get eventCopied => '已複製事件';

  @override
  String get noEvents => '沒有事件';

  @override
  String get velocity => '力度';

  @override
  String get length => '長度';

  @override
  String get velocityRange => '力度';

  @override
  String get lengthTicks => '長度';

  @override
  String get editNote => '音符';

  @override
  String get sortHint => '時間 → 音高/值 → 長度';

  @override
  String get noteLabel => '音符';

  @override
  String get markerLabel => '插入標籤';

  @override
  String get openFirst => '請先開啟 MIDI 或新建專案';

  @override
  String get exportFailed => '匯出失敗';

  @override
  String get copied => '已複製';

  @override
  String get toolSelect => '選取';

  @override
  String get toolHand => '手形';

  @override
  String get toolPencil => '鉛筆';

  @override
  String get toolEraser => '橡皮擦';

  @override
  String get toolBrush => '刷子';

  @override
  String get markerRename => '重新命名標籤';

  @override
  String get markerDelete => '刪除標籤';

  @override
  String get markerText => '標籤文字';

  @override
  String get markerTextOptional => '標籤文字（可留空）';

  @override
  String get untitled => '未命名';

  @override
  String get ppqError => 'PPQ 需為 1–32767 的整數';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return '本通道預設樂器：$name（Bank $bank / Program $program）';
  }

  @override
  String eventNoteTitle(String name) {
    return '音符 $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · 力度 $velocity · 長度 $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value（0–$max）';
  }

  @override
  String get centerHint => '8192 = 中心';

  @override
  String get jumpTo => '跳轉';

  @override
  String get exportAudio => '匯出音訊';

  @override
  String get exportMidi => '匯出 MIDI';

  @override
  String get exporting => '正在匯出…';

  @override
  String get exportDone => '匯出完成';

  @override
  String get bitDepth => '位元深度';

  @override
  String get cancelling => '正在取消…';

  @override
  String get exportFailedShort => '匯出失敗';

  @override
  String get prevPage => '上一頁';

  @override
  String get nextPage => '下一頁';

  @override
  String get importingSoundfont => '正在匯入音色庫…';

  @override
  String get importSoundfontDone => '音色庫匯入完成';

  @override
  String get noSoundfontInArchive => '壓縮檔裡沒有音色庫';

  @override
  String get loadingFile => '正在載入…';

  @override
  String get loadFailed => '載入失敗';

  @override
  String get savingProject => '正在儲存工程…';

  @override
  String get saveFailed => '儲存失敗';
}
