// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for English (`en`).
class AppLocalizationsEn extends AppLocalizations {
  AppLocalizationsEn([String locale = 'en']) : super(locale);

  @override
  String get cancel => 'Cancel';

  @override
  String get selectionActions => 'Selection actions';

  @override
  String get selectAll => 'Select all';

  @override
  String get selectAllTrack => 'Select this track';

  @override
  String get opDuplicate => 'Duplicate';

  @override
  String get opOctaveUp => 'Octave up';

  @override
  String get opOctaveDown => 'Octave down';

  @override
  String get opLengthHalf => 'Length ÷2';

  @override
  String get opLengthDouble => 'Length ×2';

  @override
  String get opFlipHorizontal => 'Flip horizontal';

  @override
  String get opFlipVertical => 'Flip vertical';

  @override
  String get opHumanize => 'Humanize';

  @override
  String get opFillGaps => 'Fill gaps';

  @override
  String get opMergeNotes => 'Merge notes';

  @override
  String get opGridSplit => 'Grid split';

  @override
  String get deleteSelectionConfirm => 'Delete selected events?';

  @override
  String get save => 'Save';

  @override
  String get ok => 'OK';

  @override
  String get back => 'Back';

  @override
  String get delete => 'Delete';

  @override
  String get edit => 'Edit';

  @override
  String get allowOverlappingNotes => 'Allow overlapping notes';

  @override
  String get copy => 'Copy';

  @override
  String get cut => 'Cut';

  @override
  String get paste => 'Paste';

  @override
  String get close => 'Close';

  @override
  String get add => 'Add';

  @override
  String get select => 'Select';

  @override
  String get clear => 'Clear';

  @override
  String get remove => 'Remove';

  @override
  String get insert => 'Insert';

  @override
  String get value => 'Value';

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
  String get openFile => 'Open file';

  @override
  String get newProject => 'New project';

  @override
  String get saveAs => 'Save as';

  @override
  String get settings => 'Settings';

  @override
  String get recentFiles => 'Recent files';

  @override
  String get noRecentFiles => 'No recent files';

  @override
  String get backToWorkspace => 'Back to workspace';

  @override
  String get home => 'Home';

  @override
  String get backToArrangement => 'Back to arrangement';

  @override
  String get play => 'Play';

  @override
  String get pause => 'Pause';

  @override
  String get stop => 'Stop';

  @override
  String get quantize => 'Quantize';

  @override
  String get quantizeOff => 'Snap off';

  @override
  String get quantizeAuto => 'Auto';

  @override
  String get tool => 'Tool';

  @override
  String get undo => 'Undo';

  @override
  String get redo => 'Redo';

  @override
  String get saveProject => 'Save project';

  @override
  String get saved => 'Saved';

  @override
  String get currentTrack => 'Current track';

  @override
  String get allTracks => 'All tracks';

  @override
  String get project => 'Project';

  @override
  String get projectSettings => 'Project settings';

  @override
  String get songTitle => 'Title';

  @override
  String get artist => 'Artist';

  @override
  String get description => 'Description';

  @override
  String get ppq => 'PPQ (rescales time when notes exist)';

  @override
  String get defaultBpm => 'Project default BPM';

  @override
  String get defaultTimeSig => 'Project default time signature';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM must be 0–60000000';

  @override
  String get timeSigField => 'Time signature (e.g. 4/4)';

  @override
  String get soundfonts => 'Soundfonts';

  @override
  String get audio => 'Audio';

  @override
  String get language => 'Language';

  @override
  String get followSystem => 'Follow system';

  @override
  String get maxVoices => 'Max voices';

  @override
  String get audioBuffer => 'Audio buffer';

  @override
  String get ignoreVelocity => 'Ignore velocity';

  @override
  String get auto => 'Auto';

  @override
  String get deviceDefault => 'Device default';

  @override
  String get addSoundfont => 'Add soundfont';

  @override
  String get frames => 'frames';

  @override
  String get editTrackInfo => 'Edit track info';

  @override
  String get eventList => 'Event list';

  @override
  String get deleteTrack => 'Delete track';

  @override
  String get trackName => 'Track name';

  @override
  String get songTitleField => 'Song title';

  @override
  String get port => 'Port';

  @override
  String get channel => 'Channel';

  @override
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Select channel default instrument';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => 'Edit master track';

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
  String get tempo => 'Tempo';

  @override
  String get timeSignature => 'Time signature';

  @override
  String get keySignature => 'Key signature';

  @override
  String get marker => 'Marker';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'Custom CC…';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

  @override
  String get customRpn => 'Custom RPN…';

  @override
  String get root => 'Root';

  @override
  String get scale => 'Scale';

  @override
  String get major => 'Major';

  @override
  String get minor => 'Natural minor';

  @override
  String get automationType => 'Automation type';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Password required';

  @override
  String get password => 'Password';

  @override
  String get open => 'Open';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Add tracks';

  @override
  String get trackCount => 'Track count (1–256)';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Paste (same time)';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

  @override
  String get velocity => 'Velocity';

  @override
  String get length => 'Length';

  @override
  String get velocityRange => 'Velocity (1–127)';

  @override
  String get lengthTicks => 'Length (ticks)';

  @override
  String get editNote => 'Edit note';

  @override
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Note';

  @override
  String get markerLabel => 'Marker';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Project export failed';

  @override
  String get copied => 'Copied';

  @override
  String get toolSelect => 'Select';

  @override
  String get toolHand => 'Hand';

  @override
  String get toolPencil => 'Pencil';

  @override
  String get toolEraser => 'Eraser';

  @override
  String get toolBrush => 'Brush';

  @override
  String get markerRename => 'Rename marker';

  @override
  String get markerDelete => 'Delete marker';

  @override
  String get markerText => 'Marker text';

  @override
  String get markerTextOptional => 'Marker text (optional)';

  @override
  String get untitled => 'Untitled';

  @override
  String get ppqError => 'PPQ must be an integer 1–32767';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return 'Channel default instrument: $name (Bank $bank / Program $program)';
  }

  @override
  String eventNoteTitle(String name) {
    return 'Note $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · vel $velocity · len $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value (0–$max)';
  }

  @override
  String get centerHint => '8192 = center';

  @override
  String get jumpTo => 'Jump';

  @override
  String get exportAudio => 'Export audio';

  @override
  String get exportMidi => 'Export MIDI';

  @override
  String get exporting => 'Exporting…';

  @override
  String get exportDone => 'Export complete';

  @override
  String get bitDepth => 'Bit depth';

  @override
  String get cancelling => 'Cancelling…';

  @override
  String get exportFailedShort => 'Export failed';

  @override
  String get prevPage => 'Previous page';

  @override
  String get nextPage => 'Next page';

  @override
  String get importingSoundfont => 'Importing soundfont…';

  @override
  String get importSoundfontDone => 'Soundfont imported';

  @override
  String get noSoundfontInArchive => 'No soundfonts in archive';

  @override
  String get loadingFile => 'Loading…';

  @override
  String get loadFailed => 'Load failed';

  @override
  String get savingProject => 'Saving project…';

  @override
  String get saveFailed => 'Save failed';

  @override
  String get unsavedChanges => 'Unsaved changes';

  @override
  String get unsavedChangesPrompt =>
      'Continuing will discard the current changes. Save first?';

  @override
  String get dontSave => 'Don\'t save';
}
