// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Russian (`ru`).
class AppLocalizationsRu extends AppLocalizations {
  AppLocalizationsRu([String locale = 'ru']) : super(locale);

  @override
  String get cancel => 'Отмена';

  @override
  String get save => 'Сохранить';

  @override
  String get ok => 'ОК';

  @override
  String get back => 'Назад';

  @override
  String get delete => 'Удалить';

  @override
  String get edit => 'Правка';

  @override
  String get copy => 'Copy';

  @override
  String get paste => 'Вставить';

  @override
  String get close => 'Закрыть';

  @override
  String get add => 'Добавить';

  @override
  String get select => 'Выделение';

  @override
  String get clear => 'Очистить';

  @override
  String get remove => 'Убрать';

  @override
  String get insert => 'Вставка';

  @override
  String get value => 'Значение';

  @override
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

  @override
  String notesCount(int count) {
    return '$count нот';
  }

  @override
  String get openFile => 'Открыть файл…';

  @override
  String get newProject => 'Новый проект';

  @override
  String get saveAs => 'Сохранить как';

  @override
  String get settings => 'Настройки';

  @override
  String get recentFiles => 'Недавно изменённые';

  @override
  String get noRecentFiles => 'Нет недавних файлов';

  @override
  String get backToWorkspace => 'Назад в рабочую область';

  @override
  String get home => 'Главная';

  @override
  String get backToArrangement => 'Назад к аранжировке';

  @override
  String get play => 'Воспроизведение';

  @override
  String get pause => 'Пауза';

  @override
  String get stop => 'Стоп';

  @override
  String get quantize => 'Квантование';

  @override
  String get quantizeOff => 'Выкл. (без привязки)';

  @override
  String get quantizeAuto => 'Авто (адаптивно)';

  @override
  String get tool => 'Инструменты';

  @override
  String get undo => 'Отменить';

  @override
  String get redo => 'Повторить';

  @override
  String get saveProject => 'Сохранить';

  @override
  String get saved => 'Проект сохранён';

  @override
  String get currentTrack => 'Текущая дорожка';

  @override
  String get allTracks => 'Показать все';

  @override
  String get project => 'Проект';

  @override
  String get projectSettings => 'Настройки проекта';

  @override
  String get songTitle => 'Название проекта';

  @override
  String get artist => 'Исполнитель';

  @override
  String get description => 'Описание';

  @override
  String get ppq =>
      'Импульсов на четвертную ноту; изменение при наличии нот вызывает пересчёт';

  @override
  String get defaultBpm => 'BPM проекта по умолчанию';

  @override
  String get defaultTimeSig => 'Размер проекта по умолчанию';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM must be 0–60000000';

  @override
  String get timeSigField => 'Тактовый размер…';

  @override
  String get soundfonts => 'SoundFont';

  @override
  String get audio => 'Аудио';

  @override
  String get language => 'Язык';

  @override
  String get followSystem => 'Как в системе';

  @override
  String get maxVoices => 'Максимум голосов';

  @override
  String get audioBuffer => 'Размер буфера';

  @override
  String get ignoreVelocity => 'Игнорировать velocity';

  @override
  String get auto => 'Авто';

  @override
  String get deviceDefault => 'По умолчанию устройства';

  @override
  String get addSoundfont => 'Добавить SoundFont';

  @override
  String get frames => 'кадров';

  @override
  String get editTrackInfo => 'Правка';

  @override
  String get eventList => 'События';

  @override
  String get deleteTrack => 'Удалить';

  @override
  String get trackName => 'Название дорожки:';

  @override
  String get songTitleField => 'Название песни:';

  @override
  String get port => 'Порт';

  @override
  String get channel => 'Канал';

  @override
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Выделение';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => 'Правка';

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
  String get tempo => 'Темп:';

  @override
  String get timeSignature => 'Тактовый размер…';

  @override
  String get keySignature => 'Тональность / лад…';

  @override
  String get marker => 'Вставить маркер';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'Свой CC';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

  @override
  String get customRpn => 'Пользовательский RPN';

  @override
  String get root => 'Тоника';

  @override
  String get scale => 'Key Signature';

  @override
  String get major => 'Мажор';

  @override
  String get minor => 'Натуральный минор';

  @override
  String get automationType => 'Автоматизация…';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Требуется пароль';

  @override
  String get password => 'Введите пароль';

  @override
  String get open => 'Открыть';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Добавить дорожку';

  @override
  String get trackCount => 'Количество';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Вставить';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

  @override
  String get velocity => 'Velocity';

  @override
  String get length => 'Gate';

  @override
  String get velocityRange => 'Velocity';

  @override
  String get lengthTicks => 'Gate';

  @override
  String get editNote => 'Ноты';

  @override
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Ноты';

  @override
  String get markerLabel => 'Вставить маркер';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Не удалось экспортировать';

  @override
  String get copied => 'Copied';

  @override
  String get toolSelect => 'Выделение';

  @override
  String get toolHand => 'Панорама';

  @override
  String get toolPencil => 'Карандаш';

  @override
  String get toolEraser => 'Ластик';

  @override
  String get toolBrush => 'Кисть';
}
