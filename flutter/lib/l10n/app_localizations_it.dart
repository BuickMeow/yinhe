// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Italian (`it`).
class AppLocalizationsIt extends AppLocalizations {
  AppLocalizationsIt([String locale = 'it']) : super(locale);

  @override
  String get cancel => 'Annulla';

  @override
  String get save => 'Salva';

  @override
  String get ok => 'OK';

  @override
  String get back => 'Indietro';

  @override
  String get delete => 'Elimina';

  @override
  String get edit => 'Modifica';

  @override
  String get copy => 'Copy';

  @override
  String get paste => 'Incolla';

  @override
  String get close => 'Chiudi';

  @override
  String get add => 'Aggiungi';

  @override
  String get select => 'Seleziona';

  @override
  String get clear => 'Cancella';

  @override
  String get remove => 'Rimuovi';

  @override
  String get insert => 'Inserisci';

  @override
  String get value => 'Valore';

  @override
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

  @override
  String notesCount(int count) {
    return '$count note';
  }

  @override
  String get openFile => 'Apri file…';

  @override
  String get newProject => 'Nuovo progetto';

  @override
  String get saveAs => 'Salva con nome';

  @override
  String get settings => 'Impostazioni';

  @override
  String get recentFiles => 'Modificati di recente';

  @override
  String get noRecentFiles => 'Nessun file recente';

  @override
  String get backToWorkspace => 'Torna allo spazio di lavoro';

  @override
  String get home => 'Home';

  @override
  String get backToArrangement => 'Torna all\'arrangiamento';

  @override
  String get play => 'Riproduzione';

  @override
  String get pause => 'Pausa';

  @override
  String get stop => 'Stop';

  @override
  String get quantize => 'Quantizzazione';

  @override
  String get quantizeOff => 'Disattivato (senza aggancio)';

  @override
  String get quantizeAuto => 'Auto (adattivo)';

  @override
  String get tool => 'Strumenti';

  @override
  String get undo => 'Annulla';

  @override
  String get redo => 'Ripeti';

  @override
  String get saveProject => 'Salva';

  @override
  String get saved => 'Progetto salvato';

  @override
  String get currentTrack => 'Traccia corrente';

  @override
  String get allTracks => 'Mostra tutto';

  @override
  String get project => 'Progetto';

  @override
  String get projectSettings => 'Impostazioni progetto';

  @override
  String get songTitle => 'Nome progetto';

  @override
  String get artist => 'Artista';

  @override
  String get description => 'Descrizione';

  @override
  String get ppq =>
      'Impulsi per quarto di nota; modificarlo con note presenti richiede una riscala';

  @override
  String get defaultBpm => 'BPM predefinito del progetto';

  @override
  String get defaultTimeSig => 'Tempo predefinito';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM must be 0–60000000';

  @override
  String get timeSigField => 'Indicazione di tempo…';

  @override
  String get soundfonts => 'SoundFont';

  @override
  String get audio => 'Audio';

  @override
  String get language => 'Lingua';

  @override
  String get followSystem => 'Segui il sistema';

  @override
  String get maxVoices => 'Voci massime';

  @override
  String get audioBuffer => 'Dimensione buffer';

  @override
  String get ignoreVelocity => 'Ignora velocity';

  @override
  String get auto => 'Automatico';

  @override
  String get deviceDefault => 'Predefinito del dispositivo';

  @override
  String get addSoundfont => 'Aggiungi SoundFont';

  @override
  String get frames => 'frame';

  @override
  String get editTrackInfo => 'Modifica';

  @override
  String get eventList => 'Eventi';

  @override
  String get deleteTrack => 'Elimina';

  @override
  String get trackName => 'Nome traccia:';

  @override
  String get songTitleField => 'Titolo del brano:';

  @override
  String get port => 'Porta';

  @override
  String get channel => 'Canale';

  @override
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Seleziona';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => 'Modifica';

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
  String get timeSignature => 'Indicazione di tempo…';

  @override
  String get keySignature => 'Tonalità / scala…';

  @override
  String get marker => 'Inserisci marcatore';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'CC personalizzato';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

  @override
  String get customRpn => 'RPN personalizzato';

  @override
  String get root => 'Fondamentale';

  @override
  String get scale => 'Key Signature';

  @override
  String get major => 'Maggiore';

  @override
  String get minor => 'Minore naturale';

  @override
  String get automationType => 'Automazione…';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Password richiesta';

  @override
  String get password => 'Inserisci password';

  @override
  String get open => 'Apri';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Aggiungi traccia';

  @override
  String get trackCount => 'Quantità';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Incolla';

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
  String get editNote => 'Note';

  @override
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Note';

  @override
  String get markerLabel => 'Inserisci marcatore';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Esportazione non riuscita';

  @override
  String get copied => 'Copied';

  @override
  String get toolSelect => 'Seleziona';

  @override
  String get toolHand => 'Pan';

  @override
  String get toolPencil => 'Matita';

  @override
  String get toolEraser => 'Gomma';

  @override
  String get toolBrush => 'Pennello';

  @override
  String get markerRename => 'Rinomina marcatore';

  @override
  String get markerDelete => 'Elimina marcatore';

  @override
  String get markerText => 'Testo del marcatore';

  @override
  String get markerTextOptional => 'Testo del marcatore (facoltativo)';
}
