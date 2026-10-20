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
  String get copy => 'Copiar';

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
  String get on => 'Activado';

  @override
  String get off => 'Desactivado';

  @override
  String get loadMore => 'Cargar más';

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
  String get quantizeAuto => 'Auto';

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
  String get bpmError => 'El BPM debe estar entre 0 y 60000000';

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
  String get frames => 'frames';

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
      'Instrumento predeterminado de este canal: sigue el PC MIDI';

  @override
  String get selectChannelInstrument => 'Seleccionar';

  @override
  String get melodicBank => 'Banco melódico 0';

  @override
  String get drumBank => 'Banco de batería 128';

  @override
  String get masterTrack => 'Editar';

  @override
  String get masterTrackHint =>
      'La pista Master (Conductor) contiene eventos globales de tempo/compás; no admite notas ni ajustes de puerto/canal.';

  @override
  String get channelInstrumentHint =>
      'Instrumento predeterminado del canal: lo comparten todas las pistas de ese canal; los eventos PC MIDI tienen prioridad.';

  @override
  String get mobilePortHint =>
      'En móvil solo se escucha el puerto A; los demás puertos se guardan para la app de escritorio.';

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
  String get customCcTitle => 'Número CC personalizado';

  @override
  String get customRpnTitle => 'Número RPN personalizado';

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
  String get tempoValueTitle => 'BPM del tempo';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = centro)';

  @override
  String get archivePasswordRequired => 'Se requiere contraseña';

  @override
  String get password => 'Introduce la contraseña';

  @override
  String get open => 'Abrir';

  @override
  String get noMidiInArchive => 'No hay archivos MIDI en el archivo';

  @override
  String get archiveOpenFailed => 'No se pudo abrir el archivo comprimido';

  @override
  String get addTracks => 'Añadir pista';

  @override
  String get trackCount => 'Cantidad';

  @override
  String get countError => 'La cantidad debe ser 1–256';

  @override
  String get cycleChannels => 'Recorrer desde el canal';

  @override
  String get sameChannel => 'Mismo canal para todos';

  @override
  String get startChannel => 'Canal inicial';

  @override
  String get pasteSameTime => 'Pegar';

  @override
  String get goToLocation => 'Ir a la ubicación';

  @override
  String get eventCopied => 'Evento copiado';

  @override
  String get noEvents => 'Sin eventos';

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
  String get sortHint => 'Tiempo → tono/valor → duración';

  @override
  String get noteLabel => 'Notas';

  @override
  String get markerLabel => 'Insertar marcador';

  @override
  String get openFirst => 'Abre un MIDI o crea un proyecto primero';

  @override
  String get exportFailed => 'Error en la exportación';

  @override
  String get copied => 'Copiado';

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

  @override
  String get markerRename => 'Cambiar nombre del marcador';

  @override
  String get markerDelete => 'Eliminar marcador';

  @override
  String get markerText => 'Texto del marcador';

  @override
  String get markerTextOptional => 'Texto del marcador (opcional)';

  @override
  String get untitled => 'Sin título';

  @override
  String get ppqError => 'El PPQ debe ser un entero de 1–32767';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return 'Instrumento predeterminado del canal: $name (Banco $bank / Programa $program)';
  }

  @override
  String eventNoteTitle(String name) {
    return 'Nota $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · velocidad $velocity · duración $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value (0–$max)';
  }

  @override
  String get centerHint => '8192 = centro';

  @override
  String get jumpTo => 'Ir a';

  @override
  String get exportAudio => 'Exportar audio';

  @override
  String get exportMidi => 'Exportar MIDI';

  @override
  String get exporting => 'Exportando…';

  @override
  String get exportDone => 'Exportación completada';

  @override
  String get bitDepth => 'Profundidad de bits';

  @override
  String get cancelling => 'Cancelando…';

  @override
  String get exportFailedShort => 'Error de exportación';
}
