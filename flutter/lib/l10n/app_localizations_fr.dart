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
  String get copy => 'Copy';

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
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

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
  String get quantizeAuto => 'Auto (adaptatif)';

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
  String get bpmError => 'BPM must be 0–60000000';

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
  String get frames => 'images';

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
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Sélection';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => 'Édition';

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
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

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
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Mot de passe requis';

  @override
  String get password => 'Saisir le mot de passe';

  @override
  String get open => 'Ouvrir';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Ajouter une piste';

  @override
  String get trackCount => 'Nombre';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Coller';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

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
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Notes';

  @override
  String get markerLabel => 'Insérer un marqueur';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Échec de l\'export';

  @override
  String get copied => 'Copied';

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
}
