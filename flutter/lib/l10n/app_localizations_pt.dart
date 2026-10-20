// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Portuguese (`pt`).
class AppLocalizationsPt extends AppLocalizations {
  AppLocalizationsPt([String locale = 'pt']) : super(locale);

  @override
  String get cancel => 'Cancelar';

  @override
  String get save => 'Salvar';

  @override
  String get ok => 'OK';

  @override
  String get back => 'Voltar';

  @override
  String get delete => 'Excluir';

  @override
  String get edit => 'Editar';

  @override
  String get copy => 'Copy';

  @override
  String get paste => 'Colar';

  @override
  String get close => 'Fechar';

  @override
  String get add => 'Adicionar';

  @override
  String get select => 'Selecionar';

  @override
  String get clear => 'Limpar';

  @override
  String get remove => 'Remover';

  @override
  String get insert => 'Inserir';

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
  String get openFile => 'Abrir arquivo…';

  @override
  String get newProject => 'Novo Projeto';

  @override
  String get saveAs => 'Salvar Como';

  @override
  String get settings => 'Configurações';

  @override
  String get recentFiles => 'Modificados Recentemente';

  @override
  String get noRecentFiles => 'Nenhum arquivo recente';

  @override
  String get backToWorkspace => 'Voltar ao espaço de trabalho';

  @override
  String get home => 'Início';

  @override
  String get backToArrangement => 'Voltar ao arranjo';

  @override
  String get play => 'Reprodução';

  @override
  String get pause => 'Pausar';

  @override
  String get stop => 'Parar';

  @override
  String get quantize => 'Quantização';

  @override
  String get quantizeOff => 'Desativado (sem snap)';

  @override
  String get quantizeAuto => 'Auto (adaptativo)';

  @override
  String get tool => 'Ferramentas';

  @override
  String get undo => 'Desfazer';

  @override
  String get redo => 'Refazer';

  @override
  String get saveProject => 'Salvar';

  @override
  String get saved => 'Projeto Salvo';

  @override
  String get currentTrack => 'Faixa atual';

  @override
  String get allTracks => 'Mostrar Tudo';

  @override
  String get project => 'Projeto';

  @override
  String get projectSettings => 'Configurações do Projeto';

  @override
  String get songTitle => 'Nome do Projeto';

  @override
  String get artist => 'Artista';

  @override
  String get description => 'Descrição';

  @override
  String get ppq =>
      'Pulsos por semínima; alterá-lo com notas presentes solicita um reescalonamento';

  @override
  String get defaultBpm => 'BPM padrão do projeto';

  @override
  String get defaultTimeSig => 'Compasso padrão do projeto';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM must be 0–60000000';

  @override
  String get timeSigField => 'Fórmula de compasso…';

  @override
  String get soundfonts => 'SoundFont';

  @override
  String get audio => 'Áudio';

  @override
  String get language => 'Idioma';

  @override
  String get followSystem => 'Seguir o sistema';

  @override
  String get maxVoices => 'Máximo de vozes';

  @override
  String get audioBuffer => 'Tamanho do Buffer';

  @override
  String get ignoreVelocity => 'Ignorar velocidade';

  @override
  String get auto => 'Automática';

  @override
  String get deviceDefault => 'Padrão do dispositivo';

  @override
  String get addSoundfont => 'Adicionar SoundFont';

  @override
  String get frames => 'quadros';

  @override
  String get editTrackInfo => 'Editar';

  @override
  String get eventList => 'Eventos';

  @override
  String get deleteTrack => 'Excluir';

  @override
  String get trackName => 'Nome da Faixa:';

  @override
  String get songTitleField => 'Título da Música:';

  @override
  String get port => 'Porta';

  @override
  String get channel => 'Canal';

  @override
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Selecionar';

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
  String get timeSignature => 'Fórmula de compasso…';

  @override
  String get keySignature => 'Tonalidade / escala…';

  @override
  String get marker => 'Inserir marcador';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'CC Personalizado';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

  @override
  String get customRpn => 'RPN personalizado';

  @override
  String get root => 'Tônica';

  @override
  String get scale => 'Key Signature';

  @override
  String get major => 'Maior';

  @override
  String get minor => 'Menor natural';

  @override
  String get automationType => 'Automação…';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Senha necessária';

  @override
  String get password => 'Digite a senha';

  @override
  String get open => 'Abrir';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Adicionar faixa';

  @override
  String get trackCount => 'Quantidade';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Colar';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

  @override
  String get velocity => 'Velocidade';

  @override
  String get length => 'Porta';

  @override
  String get velocityRange => 'Velocidade';

  @override
  String get lengthTicks => 'Porta';

  @override
  String get editNote => 'Notas';

  @override
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Notas';

  @override
  String get markerLabel => 'Inserir marcador';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Falha na Exportação';

  @override
  String get copied => 'Copied';

  @override
  String get toolSelect => 'Selecionar';

  @override
  String get toolHand => 'Panorâmica';

  @override
  String get toolPencil => 'Lápis';

  @override
  String get toolEraser => 'Borracha';

  @override
  String get toolBrush => 'Pincel';
}
