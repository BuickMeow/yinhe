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
  String get copy => 'Копировать';

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
  String get on => 'Вкл.';

  @override
  String get off => 'Выкл.';

  @override
  String get loadMore => 'Загрузить ещё';

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
  String get bpmError => 'BPM должен быть в диапазоне 0–60000000';

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
      'Инструмент этого канала по умолчанию: следует MIDI PC';

  @override
  String get selectChannelInstrument => 'Выделение';

  @override
  String get melodicBank => 'Мелодический банк 0';

  @override
  String get drumBank => 'Банк ударных 128';

  @override
  String get masterTrack => 'Правка';

  @override
  String get masterTrackHint =>
      'Дорожка Master (Conductor) содержит глобальные события темпа/размера; на ней нет нот и настроек порта/канала.';

  @override
  String get channelInstrumentHint =>
      'Инструмент канала по умолчанию: общий для всех дорожек канала; события PC из MIDI имеют приоритет.';

  @override
  String get mobilePortHint =>
      'На мобильном устройстве слышен только порт A; остальные порты сохраняются для настольного приложения.';

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
  String get customCcTitle => 'Пользовательский номер CC';

  @override
  String get customRpnTitle => 'Пользовательский номер RPN';

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
  String get tempoValueTitle => 'BPM темпа';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = центр)';

  @override
  String get archivePasswordRequired => 'Требуется пароль';

  @override
  String get password => 'Введите пароль';

  @override
  String get open => 'Открыть';

  @override
  String get noMidiInArchive => 'В архиве нет MIDI-файлов';

  @override
  String get archiveOpenFailed => 'Не удалось открыть архив';

  @override
  String get addTracks => 'Добавить дорожку';

  @override
  String get trackCount => 'Количество';

  @override
  String get countError => 'Количество должно быть 1–256';

  @override
  String get cycleChannels => 'Цикл начиная с канала';

  @override
  String get sameChannel => 'Один канал для всех';

  @override
  String get startChannel => 'Начальный канал';

  @override
  String get pasteSameTime => 'Вставить';

  @override
  String get goToLocation => 'Перейти к месту';

  @override
  String get eventCopied => 'Событие скопировано';

  @override
  String get noEvents => 'Нет событий';

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
  String get sortHint => 'Время → высота/значение → длительность';

  @override
  String get noteLabel => 'Ноты';

  @override
  String get markerLabel => 'Вставить маркер';

  @override
  String get openFirst => 'Сначала откройте MIDI или создайте проект';

  @override
  String get exportFailed => 'Не удалось экспортировать';

  @override
  String get copied => 'Скопировано';

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

  @override
  String get markerRename => 'Переименовать маркер';

  @override
  String get markerDelete => 'Удалить маркер';

  @override
  String get markerText => 'Текст маркера';

  @override
  String get markerTextOptional => 'Текст маркера (необязательно)';
}
