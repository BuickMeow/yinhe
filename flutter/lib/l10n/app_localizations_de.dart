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
  String get copy => 'Copy';

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
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

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
  String get quantizeAuto => 'Automatisch (adaptiv)';

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
  String get bpmError => 'BPM must be 0–60000000';

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
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Auswählen';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => 'Bearbeiten';

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
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

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
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Passwort erforderlich';

  @override
  String get password => 'Passwort eingeben';

  @override
  String get open => 'Öffnen';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Spur hinzufügen';

  @override
  String get trackCount => 'Anzahl';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Einfügen';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

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
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Noten';

  @override
  String get markerLabel => 'Marker einfügen';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Export fehlgeschlagen';

  @override
  String get copied => 'Copied';

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
}
