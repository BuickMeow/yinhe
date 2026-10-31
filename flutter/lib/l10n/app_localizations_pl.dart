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
  String get selectAll => 'Zaznacz wszystko';

  @override
  String get selectAllTrack => 'Zaznacz tę ścieżkę';

  @override
  String get opDuplicate => 'Duplikuj';

  @override
  String get opOctaveUp => 'Oktawa w górę';

  @override
  String get opOctaveDown => 'Oktawa w dół';

  @override
  String get opLengthHalf => 'Długość ÷2';

  @override
  String get opLengthDouble => 'Długość ×2';

  @override
  String get opFlipHorizontal => 'Odbij poziomo';

  @override
  String get opFlipVertical => 'Odbij pionowo';

  @override
  String get opHumanize => 'Humanizuj';

  @override
  String get opFillGaps => 'Wypełnij luki';

  @override
  String get opMergeNotes => 'Scal nuty';

  @override
  String get opGridSplit => 'Podziel według siatki';

  @override
  String get batchProcess => 'Edycja zbiorcza';

  @override
  String get humanizeTiming => 'Czas';

  @override
  String get humanizeVelocity => 'Dynamika';

  @override
  String get humanizeGate => 'Długość';

  @override
  String get batchVelocity => 'Dynamika / Wartość';

  @override
  String get batchGate => 'Długość';

  @override
  String get batchKey => 'Wysokość';

  @override
  String get batchTick => 'Pozycja';

  @override
  String get batchStretch => 'Rozciągnięcie';

  @override
  String get batchLength => 'Docelowa długość';

  @override
  String get batchLengthInvalid => 'Nieprawidłowa długość';

  @override
  String get deleteSelectionConfirm => 'Usunąć wybrane zdarzenia?';

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
  String get allowOverlappingNotes => 'Zezwalaj na nakładające się nuty';

  @override
  String get copy => 'Kopiuj';

  @override
  String get cut => 'Wytnij';

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
  String get on => 'Wł.';

  @override
  String get off => 'Wył.';

  @override
  String get loadMore => 'Wczytaj więcej';

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
  String get menu => 'Menu';

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
  String get quantizeAuto => 'Auto';

  @override
  String get quantizeCustom => 'Niestandardowe…';

  @override
  String get quantizeNumerator => 'Licznik';

  @override
  String get quantizeDenominator => 'Mianownik';

  @override
  String get quantizeInvalidFraction =>
      'Licznik i mianownik muszą być dodatnimi liczbami całkowitymi.';

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
  String get bpmError => 'BPM musi mieścić się w zakresie 0–60000000';

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
      'Domyślny instrument tego kanału: zgodny z MIDI PC';

  @override
  String get selectChannelInstrument => 'Zaznacz';

  @override
  String get melodicBank => 'Bank melodyczny 0';

  @override
  String get drumBank => 'Bank perkusji 128';

  @override
  String get masterTrack => 'Edycja';

  @override
  String get masterTrackHint =>
      'Ścieżka Master (Conductor) zawiera zdarzenia tempa/metryki; nie ma nut ani ustawień portu/kanału.';

  @override
  String get channelInstrumentHint =>
      'Domyślny instrument kanału: wspólny dla wszystkich ścieżek w kanale; zdarzenia PC z MIDI mają priorytet.';

  @override
  String get mobilePortHint =>
      'W wersji mobilnej słychać tylko port A; pozostałe porty są zapisywane dla aplikacji desktopowej.';

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
  String get customCcTitle => 'Niestandardowy numer CC';

  @override
  String get customRpnTitle => 'Niestandardowy numer RPN';

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
  String get automation => 'Automatyzacja';

  @override
  String get tempoValueTitle => 'BPM tempa';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = środek)';

  @override
  String get archivePasswordRequired => 'Wymagane hasło';

  @override
  String get password => 'Wprowadź hasło';

  @override
  String get open => 'Otwórz';

  @override
  String get noMidiInArchive => 'Brak plików MIDI w archiwum';

  @override
  String get archiveOpenFailed => 'Nie udało się otworzyć archiwum';

  @override
  String get addTracks => 'Dodaj ścieżkę';

  @override
  String get trackCount => 'Liczba';

  @override
  String get countError => 'Liczba musi wynosić 1–256';

  @override
  String get cycleChannels => 'Przejdź kolejno od kanału';

  @override
  String get sameChannel => 'Ten sam kanał dla wszystkich';

  @override
  String get startChannel => 'Kanał początkowy';

  @override
  String get pasteSameTime => 'Wklej';

  @override
  String get goToLocation => 'Przejdź do miejsca';

  @override
  String get eventCopied => 'Skopiowano zdarzenie';

  @override
  String get noEvents => 'Brak zdarzeń';

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
  String get sortHint => 'Czas → wysokość/wartość → długość';

  @override
  String get noteLabel => 'Nuty';

  @override
  String get markerLabel => 'Wstaw znacznik';

  @override
  String get openFirst => 'Najpierw otwórz MIDI lub utwórz projekt';

  @override
  String get exportFailed => 'Eksport nie powiódł się';

  @override
  String get copied => 'Skopiowano';

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

  @override
  String get markerRename => 'Zmień nazwę znacznika';

  @override
  String get markerDelete => 'Usuń znacznik';

  @override
  String get markerText => 'Tekst znacznika';

  @override
  String get markerTextOptional => 'Tekst znacznika (opcjonalnie)';

  @override
  String get untitled => 'Bez tytułu';

  @override
  String get ppqError => 'PPQ musi być liczbą całkowitą 1–32767';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return 'Domyślny instrument kanału: $name (bank $bank / program $program)';
  }

  @override
  String eventNoteTitle(String name) {
    return 'Nuta $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · siła $velocity · długość $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value (0–$max)';
  }

  @override
  String get centerHint => '8192 = środek';

  @override
  String get jumpTo => 'Przejdź';

  @override
  String get exportAudio => 'Eksportuj audio';

  @override
  String get exportMidi => 'Eksportuj MIDI';

  @override
  String get exporting => 'Eksportowanie…';

  @override
  String get exportDone => 'Eksport zakończony';

  @override
  String get bitDepth => 'Głębokość bitowa';

  @override
  String get cancelling => 'Anulowanie…';

  @override
  String get exportFailedShort => 'Eksport nie powiódł się';

  @override
  String get prevPage => 'Poprzednia strona';

  @override
  String get nextPage => 'Następna strona';

  @override
  String get importingSoundfont => 'Importowanie soundfontu…';

  @override
  String get importSoundfontDone => 'Soundfont zaimportowany';

  @override
  String get noSoundfontInArchive => 'Brak soundfontów w archiwum';

  @override
  String get loadingFile => 'Ładowanie…';

  @override
  String get loadFailed => 'Nie udało się wczytać';

  @override
  String get savingProject => 'Zapisywanie projektu…';

  @override
  String get saveFailed => 'Nie udało się zapisać';

  @override
  String get unsavedChanges => 'Niezapisane zmiany';

  @override
  String get unsavedChangesPrompt =>
      'Kontynuowanie odrzuci bieżące zmiany. Najpierw zapisać?';

  @override
  String get dontSave => 'Nie zapisuj';
}
