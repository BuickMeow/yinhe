// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Korean (`ko`).
class AppLocalizationsKo extends AppLocalizations {
  AppLocalizationsKo([String locale = 'ko']) : super(locale);

  @override
  String get cancel => '취소';

  @override
  String get selectAll => '모두 선택';

  @override
  String get selectAllTrack => '이 트랙 모두 선택';

  @override
  String get opDuplicate => '복제';

  @override
  String get opOctaveUp => '한 옥타브 위';

  @override
  String get opOctaveDown => '한 옥타브 아래';

  @override
  String get opLengthHalf => '길이 ÷2';

  @override
  String get opLengthDouble => '길이 ×2';

  @override
  String get opFlipHorizontal => '가로 뒤집기';

  @override
  String get opFlipVertical => '세로 뒤집기';

  @override
  String get opHumanize => '휴머나이즈';

  @override
  String get opFillGaps => '빈틈 채우기';

  @override
  String get opMergeNotes => '음표 병합';

  @override
  String get opGridSplit => '그리드로 분할';

  @override
  String get batchProcess => '일괄 처리';

  @override
  String get humanizeTiming => '타이밍';

  @override
  String get humanizeVelocity => '벨로시티';

  @override
  String get humanizeGate => '길이';

  @override
  String get batchVelocity => '벨로시티 / 값';

  @override
  String get batchGate => '길이';

  @override
  String get batchKey => '음높이';

  @override
  String get batchTick => '위치';

  @override
  String get batchStretch => '늘이기';

  @override
  String get batchLength => '목표 길이';

  @override
  String get batchLengthInvalid => '길이 형식이 잘못되었습니다';

  @override
  String get deleteSelectionConfirm => '선택한 이벤트를 삭제할까요?';

  @override
  String get save => '저장';

  @override
  String get ok => '확인';

  @override
  String get back => '뒤로';

  @override
  String get delete => '삭제';

  @override
  String get edit => '편집';

  @override
  String get allowOverlappingNotes => '음표 겹침 허용';

  @override
  String get copy => '복사';

  @override
  String get cut => '잘라내기';

  @override
  String get paste => '붙여넣기';

  @override
  String get close => '닫기';

  @override
  String get add => '추가';

  @override
  String get select => '선택';

  @override
  String get clear => '지우기';

  @override
  String get remove => '제거';

  @override
  String get insert => '삽입';

  @override
  String get value => '값';

  @override
  String get on => '켬';

  @override
  String get off => '끔';

  @override
  String get loadMore => '더 불러오기';

  @override
  String notesCount(int count) {
    return '$count 음표';
  }

  @override
  String get openFile => '파일 열기…';

  @override
  String get newProject => '새 프로젝트';

  @override
  String get saveAs => '다른 이름으로 저장';

  @override
  String get settings => '설정';

  @override
  String get recentFiles => '최근 수정한 파일';

  @override
  String get noRecentFiles => '최근 파일이 없습니다';

  @override
  String get backToWorkspace => '작업 공간으로 돌아가기';

  @override
  String get home => '홈';

  @override
  String get menu => '메뉴';

  @override
  String get backToArrangement => '어레인지로 돌아가기';

  @override
  String get play => '재생';

  @override
  String get pause => '일시정지';

  @override
  String get stop => '정지';

  @override
  String get quantize => '양자화';

  @override
  String get quantizeOff => '끄기(스냅 없음)';

  @override
  String get quantizeAuto => '자동';

  @override
  String get quantizeCustom => '사용자 지정…';

  @override
  String get quantizeNumerator => '분자';

  @override
  String get quantizeDenominator => '분모';

  @override
  String get quantizeInvalidFraction => '분자와 분모는 양의 정수여야 합니다.';

  @override
  String get tool => '도구';

  @override
  String get undo => '실행 취소';

  @override
  String get redo => '다시 실행';

  @override
  String get saveProject => '저장';

  @override
  String get saved => '프로젝트 저장됨';

  @override
  String get currentTrack => '현재 편집 트랙';

  @override
  String get allTracks => '모두 표시';

  @override
  String get project => '프로젝트';

  @override
  String get projectSettings => '프로젝트 설정';

  @override
  String get songTitle => '프로젝트 이름';

  @override
  String get artist => '아티스트';

  @override
  String get description => '설명';

  @override
  String get ppq => '4분음표당 펄스 수. 노트가 있는 상태에서 변경하면 리스케일을 확인합니다';

  @override
  String get defaultBpm => '프로젝트 기본 BPM';

  @override
  String get defaultTimeSig => '프로젝트 기본 박자';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM은 0–60000000이어야 합니다';

  @override
  String get timeSigField => '박자표…';

  @override
  String get soundfonts => '사운드폰트';

  @override
  String get audio => '오디오';

  @override
  String get language => '언어';

  @override
  String get followSystem => '시스템 설정 따르기';

  @override
  String get maxVoices => '최대 동시 발음 수';

  @override
  String get audioBuffer => '버퍼 크기';

  @override
  String get ignoreVelocity => '벨로시티 무시';

  @override
  String get auto => '자동';

  @override
  String get deviceDefault => '장치 기본값';

  @override
  String get addSoundfont => '사운드폰트 추가';

  @override
  String get frames => '프레임';

  @override
  String get editTrackInfo => '편집';

  @override
  String get eventList => '이벤트';

  @override
  String get deleteTrack => '삭제';

  @override
  String get trackName => '트랙 이름:';

  @override
  String get songTitleField => '곡 제목:';

  @override
  String get port => '포트';

  @override
  String get channel => '채널';

  @override
  String get defaultInstrumentFollowPc => '이 채널 기본 악기: MIDI PC 따름';

  @override
  String get selectChannelInstrument => '선택';

  @override
  String get melodicBank => '멜로디 Bank 0';

  @override
  String get drumBank => '드럼 Bank 128';

  @override
  String get masterTrack => '편집';

  @override
  String get masterTrackHint =>
      '마스터(Conductor) 트랙은 템포/박자 등 전역 이벤트를 담당하며, 음표를 놓을 수 없고 포트/채널 설정도 없습니다.';

  @override
  String get channelInstrumentHint =>
      '채널 기본 악기: 같은 채널의 모든 트랙이 공유합니다. MIDI PC 이벤트가 우선합니다.';

  @override
  String get mobilePortHint =>
      '모바일에서는 포트 A만 재생됩니다. 다른 포트는 저장되어 데스크톱 앱에서 사용됩니다.';

  @override
  String get tempo => '템포:';

  @override
  String get timeSignature => '박자표…';

  @override
  String get keySignature => '조표/스케일…';

  @override
  String get marker => '마커 삽입';

  @override
  String get pitchBend => '피치 벤드';

  @override
  String get customCc => '사용자 지정 CC';

  @override
  String get customCcTitle => '사용자 지정 CC 번호';

  @override
  String get customRpnTitle => '사용자 지정 RPN 번호';

  @override
  String get customRpn => '사용자 RPN';

  @override
  String get root => '근음';

  @override
  String get scale => '조표';

  @override
  String get major => '장조 Major';

  @override
  String get minor => '자연 단조 Minor';

  @override
  String get automationType => '자동화…';

  @override
  String get automation => '자동화';

  @override
  String get tempoValueTitle => '템포 BPM';

  @override
  String get pitchBendValueTitle => '피치 벤드 (0–16383, 8192 = 중앙)';

  @override
  String get archivePasswordRequired => '암호가 필요합니다';

  @override
  String get password => '비밀번호 입력';

  @override
  String get open => '열기';

  @override
  String get noMidiInArchive => '압축 파일에 MIDI 파일이 없습니다';

  @override
  String get archiveOpenFailed => '압축 파일을 열지 못했습니다';

  @override
  String get addTracks => '트랙 추가';

  @override
  String get trackCount => '개수';

  @override
  String get countError => '개수는 1–256이어야 합니다';

  @override
  String get cycleChannels => '채널부터 순환';

  @override
  String get sameChannel => '모두 같은 채널';

  @override
  String get startChannel => '시작 채널';

  @override
  String get pasteSameTime => '붙여넣기';

  @override
  String get goToLocation => '위치로 이동';

  @override
  String get eventCopied => '이벤트 복사됨';

  @override
  String get noEvents => '이벤트 없음';

  @override
  String get velocity => '벨로시티';

  @override
  String get length => '게이트';

  @override
  String get velocityRange => '벨로시티';

  @override
  String get lengthTicks => '게이트';

  @override
  String get editNote => '노트';

  @override
  String get sortHint => '시간 → 음높이/값 → 길이';

  @override
  String get noteLabel => '노트';

  @override
  String get markerLabel => '마커 삽입';

  @override
  String get openFirst => '먼저 MIDI를 열거나 프로젝트를 만드세요';

  @override
  String get exportFailed => '내보내기 실패';

  @override
  String get copied => '복사됨';

  @override
  String get toolSelect => '선택';

  @override
  String get toolHand => '손바닥';

  @override
  String get toolPencil => '연필';

  @override
  String get toolEraser => '지우개';

  @override
  String get toolBrush => '브러시';

  @override
  String get markerRename => '라벨 이름 변경';

  @override
  String get markerDelete => '라벨 삭제';

  @override
  String get markerText => '라벨 텍스트';

  @override
  String get markerTextOptional => '라벨 텍스트(비워 둘 수 있음)';

  @override
  String get untitled => '제목 없음';

  @override
  String get ppqError => 'PPQ는 1–32767의 정수여야 합니다';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return '이 채널 기본 악기: $name (Bank $bank / Program $program)';
  }

  @override
  String eventNoteTitle(String name) {
    return '음표 $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · 벨로시티 $velocity · 길이 $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value (0–$max)';
  }

  @override
  String get centerHint => '8192 = 중앙';

  @override
  String get jumpTo => '이동';

  @override
  String get exportAudio => '오디오 내보내기';

  @override
  String get exportMidi => 'MIDI 내보내기';

  @override
  String get exporting => '내보내는 중…';

  @override
  String get exportDone => '내보내기 완료';

  @override
  String get bitDepth => '비트 심도';

  @override
  String get cancelling => '취소 중…';

  @override
  String get exportFailedShort => '내보내기 실패';

  @override
  String get prevPage => '이전 페이지';

  @override
  String get nextPage => '다음 페이지';

  @override
  String get importingSoundfont => '사운드폰트 가져오는 중…';

  @override
  String get importSoundfontDone => '사운드폰트를 가져왔습니다';

  @override
  String get noSoundfontInArchive => '압축 파일에 사운드폰트가 없습니다';

  @override
  String get loadingFile => '불러오는 중…';

  @override
  String get loadFailed => '불러오기 실패';

  @override
  String get savingProject => '프로젝트 저장 중…';

  @override
  String get saveFailed => '저장 실패';

  @override
  String get unsavedChanges => '저장하지 않은 변경 사항';

  @override
  String get unsavedChangesPrompt => '계속하면 현재 변경 사항이 사라집니다. 먼저 저장할까요?';

  @override
  String get dontSave => '저장 안 함';
}
