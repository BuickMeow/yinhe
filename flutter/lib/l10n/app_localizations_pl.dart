// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Polish (`pl`).
class AppLocalizationsPl extends AppLocalizations {
  AppLocalizationsPl([String locale = 'pl']) : super(locale);

  @override
  String get cancel => 'Anuluj';

  @override
  String get save => 'Zapisz';

  @override
  String get ok => 'OK';

  @override
  String get back => 'Wstecz';

  @override
  String get delete => 'Usuń';

  @override
  String get edit => 'Edycja';

  @override
  String get copy => 'Copy';

  @override
  String get paste => 'Wklej';

  @override
  String get close => 'Zamknij';

  @override
  String get add => 'Dodaj';

  @override
  String get select => 'Zaznacz';

  @override
  String get clear => 'Wyczyść';

  @override
  String get remove => 'Usuń';

  @override
  String get insert => 'Wstaw';

  @override
  String get value => 'Wartość';

  @override
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

  @override
  String notesCount(int count) {
    return '$count nut';
  }

  @override
  String get openFile => 'Otwórz plik…';

  @override
  String get newProject => 'Nowy projekt';

  @override
  String get saveAs => 'Zapisz jako';

  @override
  String get settings => 'Ustawienia';

  @override
  String get recentFiles => 'Ostatnio modyfikowane';

  @override
  String get noRecentFiles => 'Brak ostatnich plików';

  @override
  String get backToWorkspace => 'Wróć do obszaru roboczego';

  @override
  String get home => 'Strona główna';

  @override
  String get backToArrangement => 'Wróć do aranżacji';

  @override
  String get play => 'Odtwarzanie';

  @override
  String get pause => 'Pauza';

  @override
  String get stop => 'Stop';

  @override
  String get quantize => 'Kwantyzacja';

  @override
  String get quantizeOff => 'Wyłączone (bez przyciągania)';

  @override
  String get quantizeAuto => 'Auto (adaptacyjne)';

  @override
  String get tool => 'Narzędzia';

  @override
  String get undo => 'Cofnij';

  @override
  String get redo => 'Ponów';

  @override
  String get saveProject => 'Zapisz';

  @override
  String get saved => 'Projekt zapisany';

  @override
  String get currentTrack => 'Bieżąca ścieżka';

  @override
  String get allTracks => 'Pokaż wszystko';

  @override
  String get project => 'Projekt';

  @override
  String get projectSettings => 'Ustawienia projektu';

  @override
  String get songTitle => 'Nazwa projektu';

  @override
  String get artist => 'Wykonawca';

  @override
  String get description => 'Opis';

  @override
  String get ppq =>
      'Pulsy na ćwierćnutę; zmiana przy obecnych nutach wywołuje przeskalowanie';

  @override
  String get defaultBpm => 'Domyślne BPM projektu';

  @override
  String get defaultTimeSig => 'Domyślne metrum projektu';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM must be 0–60000000';

  @override
  String get timeSigField => 'Metrum…';

  @override
  String get soundfonts => 'SoundFont';

  @override
  String get audio => 'Dźwięk';

  @override
  String get language => 'Język';

  @override
  String get followSystem => 'Zgodnie z systemem';

  @override
  String get maxVoices => 'Maksymalna liczba głosów';

  @override
  String get audioBuffer => 'Rozmiar bufora';

  @override
  String get ignoreVelocity => 'Ignoruj velocity';

  @override
  String get auto => 'Automatycznie';

  @override
  String get deviceDefault => 'Domyślne urządzenia';

  @override
  String get addSoundfont => 'Dodaj SoundFont';

  @override
  String get frames => 'ramek';

  @override
  String get editTrackInfo => 'Edycja';

  @override
  String get eventList => 'Zdarzenia';

  @override
  String get deleteTrack => 'Usuń';

  @override
  String get trackName => 'Nazwa ścieżki:';

  @override
  String get songTitleField => 'Tytuł utworu:';

  @override
  String get port => 'Port';

  @override
  String get channel => 'Kanał';

  @override
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Zaznacz';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => 'Edycja';

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
  String get tempo => 'Tempo:';

  @override
  String get timeSignature => 'Metrum…';

  @override
  String get keySignature => 'Tonacja / skala…';

  @override
  String get marker => 'Wstaw znacznik';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'Niestandardowy CC';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

  @override
  String get customRpn => 'Niestandardowy RPN';

  @override
  String get root => 'Tonika';

  @override
  String get scale => 'Key Signature';

  @override
  String get major => 'Dur';

  @override
  String get minor => 'Moll naturalny';

  @override
  String get automationType => 'Automatyzacja…';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Wymagane hasło';

  @override
  String get password => 'Wprowadź hasło';

  @override
  String get open => 'Otwórz';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Dodaj ścieżkę';

  @override
  String get trackCount => 'Liczba';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Wklej';

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
  String get editNote => 'Nuty';

  @override
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Nuty';

  @override
  String get markerLabel => 'Wstaw znacznik';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Eksport nie powiódł się';

  @override
  String get copied => 'Copied';

  @override
  String get toolSelect => 'Zaznacz';

  @override
  String get toolHand => 'Przesuwanie';

  @override
  String get toolPencil => 'Ołówek';

  @override
  String get toolEraser => 'Gumka';

  @override
  String get toolBrush => 'Pędzel';
}
