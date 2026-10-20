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
  String get copy => 'Copy';

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
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

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
  String get quantizeAuto => '자동(줌 연동)';

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
  String get bpmError => 'BPM must be 0–60000000';

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
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => '선택';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => '편집';

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
  String get tempo => '템포:';

  @override
  String get timeSignature => '박자표…';

  @override
  String get keySignature => '조표/스케일…';

  @override
  String get marker => '마커 삽입';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => '사용자 지정 CC';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

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
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => '암호가 필요합니다';

  @override
  String get password => '비밀번호 입력';

  @override
  String get open => '열기';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => '트랙 추가';

  @override
  String get trackCount => '개수';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => '붙여넣기';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

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
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => '노트';

  @override
  String get markerLabel => '마커 삽입';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => '내보내기 실패';

  @override
  String get copied => 'Copied';

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
}
