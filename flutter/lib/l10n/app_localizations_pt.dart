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
  String get selectAll => 'Selecionar tudo';

  @override
  String get selectAllTrack => 'Selecionar esta faixa';

  @override
  String get opDuplicate => 'Duplicar';

  @override
  String get opOctaveUp => 'Oitava acima';

  @override
  String get opOctaveDown => 'Oitava abaixo';

  @override
  String get opLengthHalf => 'Duração ÷2';

  @override
  String get opLengthDouble => 'Duração ×2';

  @override
  String get opFlipHorizontal => 'Inverter horizontalmente';

  @override
  String get opFlipVertical => 'Inverter verticalmente';

  @override
  String get opHumanize => 'Humanizar';

  @override
  String get opFillGaps => 'Preencher lacunas';

  @override
  String get opMergeNotes => 'Mesclar notas';

  @override
  String get opGridSplit => 'Dividir pela grade';

  @override
  String get batchProcess => 'Edição em lote';

  @override
  String get humanizeTiming => 'Tempo';

  @override
  String get humanizeVelocity => 'Velocidade';

  @override
  String get humanizeGate => 'Duração';

  @override
  String get batchVelocity => 'Velocidade / Valor';

  @override
  String get batchGate => 'Duração';

  @override
  String get batchKey => 'Nota';

  @override
  String get batchTick => 'Posição';

  @override
  String get batchStretch => 'Alongamento';

  @override
  String get batchLength => 'Comprimento alvo';

  @override
  String get batchLengthInvalid => 'Comprimento inválido';

  @override
  String get deleteSelectionConfirm => 'Excluir os eventos selecionados?';

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
  String get allowOverlappingNotes => 'Permitir notas sobrepostas';

  @override
  String get copy => 'Copiar';

  @override
  String get cut => 'Recortar';

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
  String get on => 'Ativado';

  @override
  String get off => 'Desativado';

  @override
  String get loadMore => 'Carregar mais';

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
  String get menu => 'Menu';

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
  String get quantizeAuto => 'Auto';

  @override
  String get quantizeCustom => 'Personalizado…';

  @override
  String get quantizeNumerator => 'Numerador';

  @override
  String get quantizeDenominator => 'Denominador';

  @override
  String get quantizeInvalidFraction =>
      'O numerador e o denominador devem ser números inteiros positivos.';

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
  String get bpmError => 'O BPM deve ser 0–60000000';

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
  String get frames => 'frames';

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
      'Instrumento padrão deste canal: segue o PC MIDI';

  @override
  String get selectChannelInstrument => 'Selecionar';

  @override
  String get melodicBank => 'Banco melódico 0';

  @override
  String get drumBank => 'Banco de bateria 128';

  @override
  String get masterTrack => 'Editar';

  @override
  String get masterTrackHint =>
      'A faixa Master (Conductor) contém eventos globais de tempo/compasso; não tem notas nem ajustes de porta/canal.';

  @override
  String get channelInstrumentHint =>
      'Instrumento padrão do canal: compartilhado por todas as faixas desse canal; eventos PC MIDI têm prioridade.';

  @override
  String get mobilePortHint =>
      'No celular, apenas a porta A é audível; as outras portas são salvas para o app de desktop.';

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
  String get customCcTitle => 'Número CC personalizado';

  @override
  String get customRpnTitle => 'Número RPN personalizado';

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
  String get automation => 'Automação';

  @override
  String get tempoValueTitle => 'BPM do tempo';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = centro)';

  @override
  String get archivePasswordRequired => 'Senha necessária';

  @override
  String get password => 'Digite a senha';

  @override
  String get open => 'Abrir';

  @override
  String get noMidiInArchive => 'Nenhum arquivo MIDI no pacote';

  @override
  String get archiveOpenFailed => 'Falha ao abrir o arquivo compactado';

  @override
  String get addTracks => 'Adicionar faixa';

  @override
  String get trackCount => 'Quantidade';

  @override
  String get countError => 'A quantidade deve ser 1–256';

  @override
  String get cycleChannels => 'Percorrer a partir do canal';

  @override
  String get sameChannel => 'Mesmo canal para todos';

  @override
  String get startChannel => 'Canal inicial';

  @override
  String get pasteSameTime => 'Colar';

  @override
  String get goToLocation => 'Ir para o local';

  @override
  String get eventCopied => 'Evento copiado';

  @override
  String get noEvents => 'Nenhum evento';

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
  String get sortHint => 'Tempo → altura/valor → duração';

  @override
  String get noteLabel => 'Notas';

  @override
  String get markerLabel => 'Inserir marcador';

  @override
  String get openFirst => 'Abra um MIDI ou crie um projeto primeiro';

  @override
  String get exportFailed => 'Falha na Exportação';

  @override
  String get copied => 'Copiado';

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

  @override
  String get markerRename => 'Renomear marcador';

  @override
  String get markerDelete => 'Excluir marcador';

  @override
  String get markerText => 'Texto do marcador';

  @override
  String get markerTextOptional => 'Texto do marcador (opcional)';

  @override
  String get untitled => 'Sem título';

  @override
  String get ppqError => 'O PPQ deve ser um inteiro de 1–32767';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return 'Instrumento padrão do canal: $name (Banco $bank / Programa $program)';
  }

  @override
  String eventNoteTitle(String name) {
    return 'Nota $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · velocidade $velocity · duração $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value (0–$max)';
  }

  @override
  String get centerHint => '8192 = centro';

  @override
  String get jumpTo => 'Ir para';

  @override
  String get exportAudio => 'Exportar áudio';

  @override
  String get exportMidi => 'Exportar MIDI';

  @override
  String get exporting => 'Exportando…';

  @override
  String get exportDone => 'Exportação concluída';

  @override
  String get bitDepth => 'Profundidade de bits';

  @override
  String get cancelling => 'Cancelando…';

  @override
  String get exportFailedShort => 'Falha na exportação';

  @override
  String get prevPage => 'Página anterior';

  @override
  String get nextPage => 'Próxima página';

  @override
  String get importingSoundfont => 'Importando soundfont…';

  @override
  String get importSoundfontDone => 'Soundfont importado';

  @override
  String get noSoundfontInArchive => 'Nenhum soundfont no arquivo';

  @override
  String get loadingFile => 'Carregando…';

  @override
  String get loadFailed => 'Falha ao carregar';

  @override
  String get savingProject => 'Salvando projeto…';

  @override
  String get saveFailed => 'Falha ao salvar';

  @override
  String get unsavedChanges => 'Alterações não salvas';

  @override
  String get unsavedChangesPrompt =>
      'Continuar descartará as alterações atuais. Salvar primeiro?';

  @override
  String get dontSave => 'Não salvar';
}
