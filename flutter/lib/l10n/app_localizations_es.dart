// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Spanish Castilian (`es`).
class AppLocalizationsEs extends AppLocalizations {
  AppLocalizationsEs([String locale = 'es']) : super(locale);

  @override
  String get cancel => 'Cancelar';

  @override
  String get save => 'Guardar';

  @override
  String get ok => 'Aceptar';

  @override
  String get back => 'Atrás';

  @override
  String get delete => 'Eliminar';

  @override
  String get edit => 'Editar';

  @override
  String get copy => 'Copy';

  @override
  String get paste => 'Pegar';

  @override
  String get close => 'Cerrar';

  @override
  String get add => 'Añadir';

  @override
  String get select => 'Seleccionar';

  @override
  String get clear => 'Limpiar';

  @override
  String get remove => 'Quitar';

  @override
  String get insert => 'Insertar';

  @override
  String get value => 'Valor';

  @override
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

  @override
  String notesCount(int count) {
    return '$count notas';
  }

  @override
  String get openFile => 'Abrir archivo…';

  @override
  String get newProject => 'Nuevo proyecto';

  @override
  String get saveAs => 'Guardar como';

  @override
  String get settings => 'Ajustes';

  @override
  String get recentFiles => 'Modificados recientemente';

  @override
  String get noRecentFiles => 'No hay archivos recientes';

  @override
  String get backToWorkspace => 'Volver al espacio de trabajo';

  @override
  String get home => 'Inicio';

  @override
  String get backToArrangement => 'Volver al arreglo';

  @override
  String get play => 'Reproducción';

  @override
  String get pause => 'Pausa';

  @override
  String get stop => 'Detener';

  @override
  String get quantize => 'Cuantización';

  @override
  String get quantizeOff => 'Desactivado (sin ajuste)';

  @override
  String get quantizeAuto => 'Auto (adaptativo)';

  @override
  String get tool => 'Herramientas';

  @override
  String get undo => 'Deshacer';

  @override
  String get redo => 'Rehacer';

  @override
  String get saveProject => 'Guardar';

  @override
  String get saved => 'Proyecto guardado';

  @override
  String get currentTrack => 'Pista actual';

  @override
  String get allTracks => 'Mostrar todo';

  @override
  String get project => 'Proyecto';

  @override
  String get projectSettings => 'Ajustes del proyecto';

  @override
  String get songTitle => 'Nombre del proyecto';

  @override
  String get artist => 'Artista';

  @override
  String get description => 'Descripción';

  @override
  String get ppq =>
      'Pulsos por negra; cambiarlo con notas presentes solicita un reescalado';

  @override
  String get defaultBpm => 'BPM predeterminado del proyecto';

  @override
  String get defaultTimeSig => 'Compás predeterminado';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM must be 0–60000000';

  @override
  String get timeSigField => 'Compás…';

  @override
  String get soundfonts => 'SoundFont';

  @override
  String get audio => 'Audio';

  @override
  String get language => 'Idioma';

  @override
  String get followSystem => 'Seguir el sistema';

  @override
  String get maxVoices => 'Voces máximas';

  @override
  String get audioBuffer => 'Tamaño del búfer';

  @override
  String get ignoreVelocity => 'Ignorar velocidad';

  @override
  String get auto => 'Automática';

  @override
  String get deviceDefault => 'Predeterminado del dispositivo';

  @override
  String get addSoundfont => 'Añadir SoundFont';

  @override
  String get frames => 'tramas';

  @override
  String get editTrackInfo => 'Editar';

  @override
  String get eventList => 'Eventos';

  @override
  String get deleteTrack => 'Eliminar';

  @override
  String get trackName => 'Nombre de la pista:';

  @override
  String get songTitleField => 'Título de la canción:';

  @override
  String get port => 'Puerto';

  @override
  String get channel => 'Canal';

  @override
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Seleccionar';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => 'Editar';

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
  String get timeSignature => 'Compás…';

  @override
  String get keySignature => 'Tonalidad / escala…';

  @override
  String get marker => 'Insertar marcador';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'CC personalizado';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

  @override
  String get customRpn => 'RPN personalizado';

  @override
  String get root => 'Tónica';

  @override
  String get scale => 'Key Signature';

  @override
  String get major => 'Mayor';

  @override
  String get minor => 'Menor natural';

  @override
  String get automationType => 'Automatización…';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Se requiere contraseña';

  @override
  String get password => 'Introduce la contraseña';

  @override
  String get open => 'Abrir';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Añadir pista';

  @override
  String get trackCount => 'Cantidad';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Pegar';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

  @override
  String get velocity => 'Velocidad';

  @override
  String get length => 'Duración';

  @override
  String get velocityRange => 'Velocidad';

  @override
  String get lengthTicks => 'Duración';

  @override
  String get editNote => 'Notas';

  @override
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Notas';

  @override
  String get markerLabel => 'Insertar marcador';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Error en la exportación';

  @override
  String get copied => 'Copied';

  @override
  String get toolSelect => 'Seleccionar';

  @override
  String get toolHand => 'Desplazar';

  @override
  String get toolPencil => 'Lápiz';

  @override
  String get toolEraser => 'Borrador';

  @override
  String get toolBrush => 'Pincel';
}
