// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for French (`fr`).
class AppLocalizationsFr extends AppLocalizations {
  AppLocalizationsFr([String locale = 'fr']) : super(locale);

  @override
  String get cancel => 'Annuler';

  @override
  String get selectAll => 'Tout sélectionner';

  @override
  String get selectAllTrack => 'Sélectionner cette piste';

  @override
  String get opDuplicate => 'Dupliquer';

  @override
  String get opOctaveUp => 'Octave supérieure';

  @override
  String get opOctaveDown => 'Octave inférieure';

  @override
  String get opLengthHalf => 'Durée ÷2';

  @override
  String get opLengthDouble => 'Durée ×2';

  @override
  String get opFlipHorizontal => 'Retourner horizontalement';

  @override
  String get opFlipVertical => 'Retourner verticalement';

  @override
  String get opHumanize => 'Humaniser';

  @override
  String get opFillGaps => 'Combler les vides';

  @override
  String get opMergeNotes => 'Fusionner les notes';

  @override
  String get opGridSplit => 'Diviser sur la grille';

  @override
  String get batchProcess => 'Traitement par lot';

  @override
  String get humanizeTiming => 'Timing';

  @override
  String get humanizeVelocity => 'Vélocité';

  @override
  String get humanizeGate => 'Durée';

  @override
  String get batchVelocity => 'Vélocité / Valeur';

  @override
  String get batchGate => 'Durée';

  @override
  String get batchKey => 'Hauteur';

  @override
  String get batchTick => 'Position';

  @override
  String get batchStretch => 'Étirement';

  @override
  String get batchLength => 'Longueur cible';

  @override
  String get batchLengthInvalid => 'Longueur non valide';

  @override
  String get deleteSelectionConfirm =>
      'Supprimer les événements sélectionnés ?';

  @override
  String get save => 'Enregistrer';

  @override
  String get ok => 'OK';

  @override
  String get back => 'Retour';

  @override
  String get delete => 'Supprimer';

  @override
  String get edit => 'Édition';

  @override
  String get allowOverlappingNotes => 'Autoriser le chevauchement des notes';

  @override
  String get copy => 'Copier';

  @override
  String get cut => 'Couper';

  @override
  String get paste => 'Coller';

  @override
  String get close => 'Fermer';

  @override
  String get add => 'Ajouter';

  @override
  String get select => 'Sélection';

  @override
  String get clear => 'Effacer';

  @override
  String get remove => 'Retirer';

  @override
  String get insert => 'Insertion';

  @override
  String get value => 'Valeur';

  @override
  String get on => 'Activé';

  @override
  String get off => 'Désactivé';

  @override
  String get loadMore => 'Charger plus';

  @override
  String notesCount(int count) {
    return '$count notes';
  }

  @override
  String get openFile => 'Ouvrir un fichier…';

  @override
  String get newProject => 'Nouveau projet';

  @override
  String get saveAs => 'Enregistrer sous';

  @override
  String get settings => 'Paramètres';

  @override
  String get recentFiles => 'Récemment modifiés';

  @override
  String get noRecentFiles => 'Aucun fichier récent';

  @override
  String get backToWorkspace => 'Retour à l\'espace de travail';

  @override
  String get home => 'Accueil';

  @override
  String get menu => 'Menu';

  @override
  String get backToArrangement => 'Retour à l\'arrangement';

  @override
  String get play => 'Lecture';

  @override
  String get pause => 'Pause';

  @override
  String get stop => 'Stop';

  @override
  String get quantize => 'Quantification';

  @override
  String get quantizeOff => 'Désactivé (sans aimantation)';

  @override
  String get quantizeAuto => 'Auto';

  @override
  String get quantizeCustom => 'Personnalisé…';

  @override
  String get quantizeNumerator => 'Numérateur';

  @override
  String get quantizeDenominator => 'Dénominateur';

  @override
  String get quantizeInvalidFraction =>
      'Le numérateur et le dénominateur doivent être des entiers positifs.';

  @override
  String get tool => 'Outils';

  @override
  String get undo => 'Annuler';

  @override
  String get redo => 'Rétablir';

  @override
  String get saveProject => 'Enregistrer';

  @override
  String get saved => 'Projet enregistré';

  @override
  String get currentTrack => 'Piste en cours';

  @override
  String get allTracks => 'Tout afficher';

  @override
  String get project => 'Projet';

  @override
  String get projectSettings => 'Paramètres du projet';

  @override
  String get songTitle => 'Nom du projet';

  @override
  String get artist => 'Artiste';

  @override
  String get description => 'Description';

  @override
  String get ppq =>
      'Pulsations par noire ; la modifier alors que des notes existent déclenche une remise à l\'échelle';

  @override
  String get defaultBpm => 'BPM par défaut du projet';

  @override
  String get defaultTimeSig => 'Signature rythmique par défaut';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'Le BPM doit être compris entre 0 et 60000000';

  @override
  String get timeSigField => 'Signature rythmique…';

  @override
  String get soundfonts => 'SoundFont';

  @override
  String get audio => 'Audio';

  @override
  String get language => 'Langue';

  @override
  String get followSystem => 'Suivre le système';

  @override
  String get maxVoices => 'Voix maximum';

  @override
  String get audioBuffer => 'Taille du tampon';

  @override
  String get ignoreVelocity => 'Ignorer la vélocité';

  @override
  String get auto => 'Auto';

  @override
  String get deviceDefault => 'Par défaut de l\'appareil';

  @override
  String get addSoundfont => 'Ajouter une SoundFont';

  @override
  String get frames => 'frames';

  @override
  String get editTrackInfo => 'Édition';

  @override
  String get eventList => 'Événements';

  @override
  String get deleteTrack => 'Supprimer';

  @override
  String get trackName => 'Nom de la piste :';

  @override
  String get songTitleField => 'Titre du morceau :';

  @override
  String get port => 'Port';

  @override
  String get channel => 'Canal';

  @override
  String get defaultInstrumentFollowPc =>
      'Instrument par défaut de ce canal : suit le PC MIDI';

  @override
  String get selectChannelInstrument => 'Sélection';

  @override
  String get melodicBank => 'Banque mélodique 0';

  @override
  String get drumBank => 'Banque de batterie 128';

  @override
  String get masterTrack => 'Édition';

  @override
  String get masterTrackHint =>
      'La piste Master (Conductor) contient les événements de tempo/métrique ; elle n\'a ni notes ni port/canal.';

  @override
  String get channelInstrumentHint =>
      'Instrument par défaut du canal : partagé par toutes les pistes de ce canal ; les événements PC MIDI sont prioritaires.';

  @override
  String get mobilePortHint =>
      'Sur mobile, seul le port A est audible ; les autres ports sont enregistrés pour l\'application de bureau.';

  @override
  String get tempo => 'Tempo :';

  @override
  String get timeSignature => 'Signature rythmique…';

  @override
  String get keySignature => 'Tonalité / mode…';

  @override
  String get marker => 'Insérer un marqueur';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'CC personnalisé';

  @override
  String get customCcTitle => 'Numéro CC personnalisé';

  @override
  String get customRpnTitle => 'Numéro RPN personnalisé';

  @override
  String get customRpn => 'RPN personnalisé';

  @override
  String get root => 'Fondamentale';

  @override
  String get scale => 'Key Signature';

  @override
  String get major => 'Majeur';

  @override
  String get minor => 'Mineur naturel';

  @override
  String get automationType => 'Automatisation…';

  @override
  String get automation => 'Automatisation';

  @override
  String get tempoValueTitle => 'BPM du tempo';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = centre)';

  @override
  String get archivePasswordRequired => 'Mot de passe requis';

  @override
  String get password => 'Saisir le mot de passe';

  @override
  String get open => 'Ouvrir';

  @override
  String get noMidiInArchive => 'Aucun fichier MIDI dans l\'archive';

  @override
  String get archiveOpenFailed => 'Échec de l\'ouverture de l\'archive';

  @override
  String get addTracks => 'Ajouter une piste';

  @override
  String get trackCount => 'Nombre';

  @override
  String get countError => 'Le nombre doit être compris entre 1 et 256';

  @override
  String get cycleChannels => 'Parcourir à partir du canal';

  @override
  String get sameChannel => 'Même canal pour tous';

  @override
  String get startChannel => 'Canal de départ';

  @override
  String get pasteSameTime => 'Coller';

  @override
  String get goToLocation => 'Aller à l\'emplacement';

  @override
  String get eventCopied => 'Événement copié';

  @override
  String get noEvents => 'Aucun événement';

  @override
  String get velocity => 'Vélocité';

  @override
  String get length => 'Gate';

  @override
  String get velocityRange => 'Vélocité';

  @override
  String get lengthTicks => 'Gate';

  @override
  String get editNote => 'Notes';

  @override
  String get sortHint => 'Temps → hauteur/valeur → durée';

  @override
  String get noteLabel => 'Notes';

  @override
  String get markerLabel => 'Insérer un marqueur';

  @override
  String get openFirst => 'Ouvrez d\'abord un MIDI ou créez un projet';

  @override
  String get exportFailed => 'Échec de l\'export';

  @override
  String get copied => 'Copié';

  @override
  String get toolSelect => 'Sélection';

  @override
  String get toolHand => 'Déplacer';

  @override
  String get toolPencil => 'Crayon';

  @override
  String get toolEraser => 'Gomme';

  @override
  String get toolBrush => 'Pinceau';

  @override
  String get markerRename => 'Renommer le marqueur';

  @override
  String get markerDelete => 'Supprimer le marqueur';

  @override
  String get markerText => 'Texte du marqueur';

  @override
  String get markerTextOptional => 'Texte du marqueur (facultatif)';

  @override
  String get untitled => 'Sans titre';

  @override
  String get ppqError => 'Le PPQ doit être un entier entre 1 et 32767';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return 'Instrument par défaut du canal : $name (Banque $bank / Programme $program)';
  }

  @override
  String eventNoteTitle(String name) {
    return 'Note $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · vélocité $velocity · durée $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value (0–$max)';
  }

  @override
  String get centerHint => '8192 = centre';

  @override
  String get jumpTo => 'Aller à';

  @override
  String get exportAudio => 'Exporter l\'audio';

  @override
  String get exportMidi => 'Exporter le MIDI';

  @override
  String get exporting => 'Export en cours…';

  @override
  String get exportDone => 'Export terminé';

  @override
  String get bitDepth => 'Profondeur de bits';

  @override
  String get cancelling => 'Annulation…';

  @override
  String get exportFailedShort => 'Échec de l\'export';

  @override
  String get prevPage => 'Page précédente';

  @override
  String get nextPage => 'Page suivante';

  @override
  String get importingSoundfont => 'Import du soundfont…';

  @override
  String get importSoundfontDone => 'Soundfont importé';

  @override
  String get noSoundfontInArchive => 'Aucun soundfont dans l\'archive';

  @override
  String get loadingFile => 'Chargement…';

  @override
  String get loadFailed => 'Échec du chargement';

  @override
  String get savingProject => 'Enregistrement du projet…';

  @override
  String get saveFailed => 'Échec de l’enregistrement';

  @override
  String get unsavedChanges => 'Modifications non enregistrées';

  @override
  String get unsavedChangesPrompt =>
      'Continuer effacera les modifications actuelles. Enregistrer d’abord ?';

  @override
  String get dontSave => 'Ne pas enregistrer';
}
