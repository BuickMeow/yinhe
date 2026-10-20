// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Czech (`cs`).
class AppLocalizationsCs extends AppLocalizations {
  AppLocalizationsCs([String locale = 'cs']) : super(locale);

  @override
  String get cancel => 'Zrušit';

  @override
  String get save => 'Uložit';

  @override
  String get ok => 'OK';

  @override
  String get back => 'Zpět';

  @override
  String get delete => 'Smazat';

  @override
  String get edit => 'Úpravy';

  @override
  String get copy => 'Copy';

  @override
  String get paste => 'Vložit';

  @override
  String get close => 'Zavřít';

  @override
  String get add => 'Přidat';

  @override
  String get select => 'Výběr';

  @override
  String get clear => 'Vymazat';

  @override
  String get remove => 'Odebrat';

  @override
  String get insert => 'Vložit';

  @override
  String get value => 'Hodnota';

  @override
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

  @override
  String notesCount(int count) {
    return '$count not';
  }

  @override
  String get openFile => 'Otevřít soubor…';

  @override
  String get newProject => 'Nový projekt';

  @override
  String get saveAs => 'Uložit jako';

  @override
  String get settings => 'Nastavení';

  @override
  String get recentFiles => 'Nedávno upravené';

  @override
  String get noRecentFiles => 'Žádné nedávné soubory';

  @override
  String get backToWorkspace => 'Zpět do pracovního prostoru';

  @override
  String get home => 'Domů';

  @override
  String get backToArrangement => 'Zpět do aranžmá';

  @override
  String get play => 'Přehrávání';

  @override
  String get pause => 'Pozastavit';

  @override
  String get stop => 'Zastavit';

  @override
  String get quantize => 'Kvantizace';

  @override
  String get quantizeOff => 'Vypnuto (bez přichytávání)';

  @override
  String get quantizeAuto => 'Automaticky (adaptivní)';

  @override
  String get tool => 'Nástroje';

  @override
  String get undo => 'Zpět';

  @override
  String get redo => 'Znovu';

  @override
  String get saveProject => 'Uložit';

  @override
  String get saved => 'Projekt uložen';

  @override
  String get currentTrack => 'Aktuální stopa';

  @override
  String get allTracks => 'Zobrazit vše';

  @override
  String get project => 'Projekt';

  @override
  String get projectSettings => 'Nastavení projektu';

  @override
  String get songTitle => 'Název projektu';

  @override
  String get artist => 'Interpret';

  @override
  String get description => 'Popis';

  @override
  String get ppq =>
      'Pulsů na čtvrtovou notu; změna při existujících notách vyvolá přepočet';

  @override
  String get defaultBpm => 'Výchozí BPM projektu';

  @override
  String get defaultTimeSig => 'Výchozí takt projektu';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM must be 0–60000000';

  @override
  String get timeSigField => 'Taktové označení…';

  @override
  String get soundfonts => 'SoundFont';

  @override
  String get audio => 'Zvuk';

  @override
  String get language => 'Jazyk';

  @override
  String get followSystem => 'Podle systému';

  @override
  String get maxVoices => 'Maximální počet hlasů';

  @override
  String get audioBuffer => 'Velikost vyrovnávací paměti';

  @override
  String get ignoreVelocity => 'Ignorovat dynamiku';

  @override
  String get auto => 'Automaticky';

  @override
  String get deviceDefault => 'Výchozí zařízení';

  @override
  String get addSoundfont => 'Přidat SoundFont';

  @override
  String get frames => 'snímků';

  @override
  String get editTrackInfo => 'Úpravy';

  @override
  String get eventList => 'Události';

  @override
  String get deleteTrack => 'Smazat';

  @override
  String get trackName => 'Název stopy:';

  @override
  String get songTitleField => 'Název skladby:';

  @override
  String get port => 'Port';

  @override
  String get channel => 'Kanál';

  @override
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Výběr';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => 'Úpravy';

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
  String get timeSignature => 'Taktové označení…';

  @override
  String get keySignature => 'Tónina / stupnice…';

  @override
  String get marker => 'Vložit značku';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'Vlastní CC';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

  @override
  String get customRpn => 'Vlastní RPN';

  @override
  String get root => 'Tónika';

  @override
  String get scale => 'Key Signature';

  @override
  String get major => 'Dur';

  @override
  String get minor => 'Přirozený moll';

  @override
  String get automationType => 'Automatizace…';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Vyžadováno heslo';

  @override
  String get password => 'Zadejte heslo';

  @override
  String get open => 'Otevřít';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Přidat stopu';

  @override
  String get trackCount => 'Počet';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Vložit';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

  @override
  String get velocity => 'Dynamika';

  @override
  String get length => 'Délka noty';

  @override
  String get velocityRange => 'Dynamika';

  @override
  String get lengthTicks => 'Délka noty';

  @override
  String get editNote => 'Noty';

  @override
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Noty';

  @override
  String get markerLabel => 'Vložit značku';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Export selhal';

  @override
  String get copied => 'Copied';

  @override
  String get toolSelect => 'Výběr';

  @override
  String get toolHand => 'Posun';

  @override
  String get toolPencil => 'Tužka';

  @override
  String get toolEraser => 'Guma';

  @override
  String get toolBrush => 'Štětec';
}
