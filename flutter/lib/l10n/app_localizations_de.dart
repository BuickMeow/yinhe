// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for German (`de`).
class AppLocalizationsDe extends AppLocalizations {
  AppLocalizationsDe([String locale = 'de']) : super(locale);

  @override
  String get cancel => 'Abbrechen';

  @override
  String get save => 'Speichern';

  @override
  String get ok => 'OK';

  @override
  String get back => 'Zurück';

  @override
  String get delete => 'Löschen';

  @override
  String get edit => 'Bearbeiten';

  @override
  String get allowOverlappingNotes => 'Überlappende Noten zulassen';

  @override
  String get copy => 'Kopieren';

  @override
  String get paste => 'Einfügen';

  @override
  String get close => 'Schließen';

  @override
  String get add => 'Hinzufügen';

  @override
  String get select => 'Auswählen';

  @override
  String get clear => 'Leeren';

  @override
  String get remove => 'Entfernen';

  @override
  String get insert => 'Einfügen';

  @override
  String get value => 'Wert';

  @override
  String get on => 'Ein';

  @override
  String get off => 'Aus';

  @override
  String get loadMore => 'Mehr laden';

  @override
  String notesCount(int count) {
    return '$count Noten';
  }

  @override
  String get openFile => 'Datei öffnen…';

  @override
  String get newProject => 'Neues Projekt';

  @override
  String get saveAs => 'Speichern unter';

  @override
  String get settings => 'Einstellungen';

  @override
  String get recentFiles => 'Zuletzt geändert';

  @override
  String get noRecentFiles => 'Keine zuletzt geöffneten Dateien';

  @override
  String get backToWorkspace => 'Zurück zum Arbeitsbereich';

  @override
  String get home => 'Startseite';

  @override
  String get backToArrangement => 'Zurück zum Arrangement';

  @override
  String get play => 'Wiedergabe';

  @override
  String get pause => 'Pause';

  @override
  String get stop => 'Stopp';

  @override
  String get quantize => 'Quantisierung';

  @override
  String get quantizeOff => 'Aus (ohne Einrasten)';

  @override
  String get quantizeAuto => 'Automatisch';

  @override
  String get tool => 'Werkzeuge';

  @override
  String get undo => 'Rückgängig';

  @override
  String get redo => 'Wiederholen';

  @override
  String get saveProject => 'Speichern';

  @override
  String get saved => 'Projekt gespeichert';

  @override
  String get currentTrack => 'Aktuelle Spur';

  @override
  String get allTracks => 'Alle einblenden';

  @override
  String get project => 'Projekt';

  @override
  String get projectSettings => 'Projekteinstellungen';

  @override
  String get songTitle => 'Projektname';

  @override
  String get artist => 'Künstler';

  @override
  String get description => 'Beschreibung';

  @override
  String get ppq =>
      'Pulse pro Viertelnote; eine Änderung bei vorhandenen Noten löst eine Neuskalierung aus';

  @override
  String get defaultBpm => 'Projekt-Standard-BPM';

  @override
  String get defaultTimeSig => 'Projekt-Standard-Taktart';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM muss 0–60000000 sein';

  @override
  String get timeSigField => 'Taktart…';

  @override
  String get soundfonts => 'SoundFont';

  @override
  String get audio => 'Audio';

  @override
  String get language => 'Sprache';

  @override
  String get followSystem => 'Systemeinstellung folgen';

  @override
  String get maxVoices => 'Maximale Stimmen';

  @override
  String get audioBuffer => 'Puffergröße';

  @override
  String get ignoreVelocity => 'Anschlagstärke ignorieren';

  @override
  String get auto => 'Automatisch';

  @override
  String get deviceDefault => 'Gerätestandard';

  @override
  String get addSoundfont => 'SoundFont hinzufügen';

  @override
  String get frames => 'Frames';

  @override
  String get editTrackInfo => 'Bearbeiten';

  @override
  String get eventList => 'Ereignisse';

  @override
  String get deleteTrack => 'Löschen';

  @override
  String get trackName => 'Spurname:';

  @override
  String get songTitleField => 'Songtitel:';

  @override
  String get port => 'Port';

  @override
  String get channel => 'Kanal';

  @override
  String get defaultInstrumentFollowPc =>
      'Standardinstrument dieses Kanals: folgt MIDI-PC';

  @override
  String get selectChannelInstrument => 'Auswählen';

  @override
  String get melodicBank => 'Melodie-Bank 0';

  @override
  String get drumBank => 'Drum-Bank 128';

  @override
  String get masterTrack => 'Bearbeiten';

  @override
  String get masterTrackHint =>
      'Die Master-(Conductor-)Spur enthält Tempo-/Taktart-Events; sie hat keine Noten und keine Port-/Kanal-Einstellungen.';

  @override
  String get channelInstrumentHint =>
      'Standardinstrument des Kanals: wird von allen Spuren dieses Kanals geteilt; MIDI-PC-Events haben Vorrang.';

  @override
  String get mobilePortHint =>
      'Auf Mobilgeräten ist nur Port A hörbar; andere Ports werden für die Desktop-App gespeichert.';

  @override
  String get tempo => 'Tempo:';

  @override
  String get timeSignature => 'Taktart…';

  @override
  String get keySignature => 'Tonart / Tonleiter…';

  @override
  String get marker => 'Marker einfügen';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'Benutzerdefiniertes CC';

  @override
  String get customCcTitle => 'Benutzerdefinierte CC-Nummer';

  @override
  String get customRpnTitle => 'Benutzerdefinierte RPN-Nummer';

  @override
  String get customRpn => 'Benutzerdefiniertes RPN';

  @override
  String get root => 'Grundton';

  @override
  String get scale => 'Key Signature';

  @override
  String get major => 'Dur';

  @override
  String get minor => 'Natürliches Moll';

  @override
  String get automationType => 'Automation…';

  @override
  String get tempoValueTitle => 'Tempo-BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = Mitte)';

  @override
  String get archivePasswordRequired => 'Passwort erforderlich';

  @override
  String get password => 'Passwort eingeben';

  @override
  String get open => 'Öffnen';

  @override
  String get noMidiInArchive => 'Keine MIDI-Dateien im Archiv';

  @override
  String get archiveOpenFailed => 'Archiv konnte nicht geöffnet werden';

  @override
  String get addTracks => 'Spur hinzufügen';

  @override
  String get trackCount => 'Anzahl';

  @override
  String get countError => 'Anzahl muss 1–256 sein';

  @override
  String get cycleChannels => 'Ab Kanal durchlaufen';

  @override
  String get sameChannel => 'Gleicher Kanal für alle';

  @override
  String get startChannel => 'Startkanal';

  @override
  String get pasteSameTime => 'Einfügen';

  @override
  String get goToLocation => 'Zur Position springen';

  @override
  String get eventCopied => 'Event kopiert';

  @override
  String get noEvents => 'Keine Events';

  @override
  String get velocity => 'Anschlagstärke';

  @override
  String get length => 'Gate';

  @override
  String get velocityRange => 'Anschlagstärke';

  @override
  String get lengthTicks => 'Gate';

  @override
  String get editNote => 'Noten';

  @override
  String get sortHint => 'Zeit → Tonhöhe/Wert → Länge';

  @override
  String get noteLabel => 'Noten';

  @override
  String get markerLabel => 'Marker einfügen';

  @override
  String get openFirst => 'Zuerst MIDI öffnen oder ein Projekt erstellen';

  @override
  String get exportFailed => 'Export fehlgeschlagen';

  @override
  String get copied => 'Kopiert';

  @override
  String get toolSelect => 'Auswählen';

  @override
  String get toolHand => 'Verschieben';

  @override
  String get toolPencil => 'Stift';

  @override
  String get toolEraser => 'Radierer';

  @override
  String get toolBrush => 'Pinsel';

  @override
  String get markerRename => 'Marker umbenennen';

  @override
  String get markerDelete => 'Marker löschen';

  @override
  String get markerText => 'Markertext';

  @override
  String get markerTextOptional => 'Markertext (optional)';

  @override
  String get untitled => 'Unbenannt';

  @override
  String get ppqError => 'PPQ muss eine ganze Zahl von 1–32767 sein';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return 'Standardinstrument des Kanals: $name (Bank $bank / Program $program)';
  }

  @override
  String eventNoteTitle(String name) {
    return 'Note $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · Anschlag $velocity · Länge $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value (0–$max)';
  }

  @override
  String get centerHint => '8192 = Mitte';

  @override
  String get jumpTo => 'Springen';

  @override
  String get exportAudio => 'Audio exportieren';

  @override
  String get exportMidi => 'MIDI exportieren';

  @override
  String get exporting => 'Exportiere…';

  @override
  String get exportDone => 'Export abgeschlossen';

  @override
  String get bitDepth => 'Bittiefe';

  @override
  String get cancelling => 'Wird abgebrochen…';

  @override
  String get exportFailedShort => 'Export fehlgeschlagen';

  @override
  String get prevPage => 'Vorherige Seite';

  @override
  String get nextPage => 'Nächste Seite';

  @override
  String get importingSoundfont => 'Soundfont wird importiert…';

  @override
  String get importSoundfontDone => 'Soundfont importiert';

  @override
  String get noSoundfontInArchive => 'Keine Soundfonts im Archiv';

  @override
  String get loadingFile => 'Wird geladen…';

  @override
  String get loadFailed => 'Laden fehlgeschlagen';

  @override
  String get savingProject => 'Projekt wird gespeichert…';

  @override
  String get saveFailed => 'Speichern fehlgeschlagen';

  @override
  String get unsavedChanges => 'Ungespeicherte Änderungen';

  @override
  String get unsavedChangesPrompt =>
      'Fortfahren verwirft die aktuellen Änderungen. Zuerst speichern?';

  @override
  String get dontSave => 'Nicht speichern';
}
