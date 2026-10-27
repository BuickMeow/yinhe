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
  String get selectAll => 'Seleziona tutto';

  @override
  String get selectAllTrack => 'Seleziona questa traccia';

  @override
  String get opDuplicate => 'Duplica';

  @override
  String get opOctaveUp => 'Ottava su';

  @override
  String get opOctaveDown => 'Ottava giù';

  @override
  String get opLengthHalf => 'Durata ÷2';

  @override
  String get opLengthDouble => 'Durata ×2';

  @override
  String get opFlipHorizontal => 'Rifletti orizzontale';

  @override
  String get opFlipVertical => 'Rifletti verticale';

  @override
  String get opHumanize => 'Umanizza';

  @override
  String get opFillGaps => 'Riempi spazi';

  @override
  String get opMergeNotes => 'Unisci note';

  @override
  String get opGridSplit => 'Dividi sulla griglia';

  @override
  String get batchProcess => 'Modifica in blocco';

  @override
  String get humanizeTiming => 'Timing';

  @override
  String get humanizeVelocity => 'Velocità';

  @override
  String get humanizeGate => 'Durata';

  @override
  String get batchVelocity => 'Velocità / Valore';

  @override
  String get batchGate => 'Durata';

  @override
  String get batchKey => 'Altezza';

  @override
  String get batchTick => 'Posizione';

  @override
  String get batchStretch => 'Estensione';

  @override
  String get batchExprHint => 'Supporta 100, +2, x1.5, 3/7';

  @override
  String get deleteSelectionConfirm => 'Eliminare gli eventi selezionati?';

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
  String get allowOverlappingNotes => 'Consenti note sovrapposte';

  @override
  String get copy => 'Copia';

  @override
  String get cut => 'Taglia';

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
  String get on => 'Attivato';

  @override
  String get off => 'Disattivato';

  @override
  String get loadMore => 'Carica altro';

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
  String get quantizeAuto => 'Auto';

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
  String get bpmError => 'Il BPM deve essere 0–60000000';

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
      'Strumento predefinito di questo canale: segue il PC MIDI';

  @override
  String get selectChannelInstrument => 'Seleziona';

  @override
  String get melodicBank => 'Banco melodico 0';

  @override
  String get drumBank => 'Banco batteria 128';

  @override
  String get masterTrack => 'Modifica';

  @override
  String get masterTrackHint =>
      'La traccia Master (Conductor) contiene eventi globali di tempo/misura; non ha note né impostazioni di porta/canale.';

  @override
  String get channelInstrumentHint =>
      'Strumento predefinito del canale: condiviso da tutte le tracce dello stesso canale; gli eventi PC MIDI hanno priorità.';

  @override
  String get mobilePortHint =>
      'Su mobile è udibile solo la porta A; le altre porte vengono salvate per l\'app desktop.';

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
  String get customCcTitle => 'Numero CC personalizzato';

  @override
  String get customRpnTitle => 'Numero RPN personalizzato';

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
  String get tempoValueTitle => 'BPM del tempo';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = centro)';

  @override
  String get archivePasswordRequired => 'Password richiesta';

  @override
  String get password => 'Inserisci password';

  @override
  String get open => 'Apri';

  @override
  String get noMidiInArchive => 'Nessun file MIDI nell\'archivio';

  @override
  String get archiveOpenFailed => 'Impossibile aprire l\'archivio';

  @override
  String get addTracks => 'Aggiungi traccia';

  @override
  String get trackCount => 'Quantità';

  @override
  String get countError => 'La quantità deve essere 1–256';

  @override
  String get cycleChannels => 'Ciclo dal canale';

  @override
  String get sameChannel => 'Stesso canale per tutti';

  @override
  String get startChannel => 'Canale iniziale';

  @override
  String get pasteSameTime => 'Incolla';

  @override
  String get goToLocation => 'Vai alla posizione';

  @override
  String get eventCopied => 'Evento copiato';

  @override
  String get noEvents => 'Nessun evento';

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
  String get sortHint => 'Tempo → altezza/valore → durata';

  @override
  String get noteLabel => 'Note';

  @override
  String get markerLabel => 'Inserisci marcatore';

  @override
  String get openFirst => 'Apri prima un MIDI o crea un progetto';

  @override
  String get exportFailed => 'Esportazione non riuscita';

  @override
  String get copied => 'Copiato';

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

  @override
  String get untitled => 'Senza titolo';

  @override
  String get ppqError => 'Il PPQ deve essere un intero 1–32767';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return 'Strumento predefinito del canale: $name (Bank $bank / Program $program)';
  }

  @override
  String eventNoteTitle(String name) {
    return 'Nota $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · velocity $velocity · durata $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value (0–$max)';
  }

  @override
  String get centerHint => '8192 = centro';

  @override
  String get jumpTo => 'Vai a';

  @override
  String get exportAudio => 'Esporta audio';

  @override
  String get exportMidi => 'Esporta MIDI';

  @override
  String get exporting => 'Esportazione…';

  @override
  String get exportDone => 'Esportazione completata';

  @override
  String get bitDepth => 'Profondità di bit';

  @override
  String get cancelling => 'Annullamento…';

  @override
  String get exportFailedShort => 'Esportazione non riuscita';

  @override
  String get prevPage => 'Pagina precedente';

  @override
  String get nextPage => 'Pagina successiva';

  @override
  String get importingSoundfont => 'Importazione soundfont…';

  @override
  String get importSoundfontDone => 'Soundfont importato';

  @override
  String get noSoundfontInArchive => 'Nessun soundfont nell\'archivio';

  @override
  String get loadingFile => 'Caricamento…';

  @override
  String get loadFailed => 'Caricamento non riuscito';

  @override
  String get savingProject => 'Salvataggio progetto…';

  @override
  String get saveFailed => 'Salvataggio non riuscito';

  @override
  String get unsavedChanges => 'Modifiche non salvate';

  @override
  String get unsavedChangesPrompt =>
      'Continuando le modifiche attuali andranno perse. Salvare prima?';

  @override
  String get dontSave => 'Non salvare';
}
