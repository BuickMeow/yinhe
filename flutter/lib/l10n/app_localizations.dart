import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:intl/intl.dart' as intl;

import 'app_localizations_cs.dart';
import 'app_localizations_de.dart';
import 'app_localizations_en.dart';
import 'app_localizations_es.dart';
import 'app_localizations_fr.dart';
import 'app_localizations_it.dart';
import 'app_localizations_ja.dart';
import 'app_localizations_ko.dart';
import 'app_localizations_pl.dart';
import 'app_localizations_pt.dart';
import 'app_localizations_ru.dart';
import 'app_localizations_tr.dart';
import 'app_localizations_zh.dart';

// ignore_for_file: type=lint

/// Callers can lookup localized strings with an instance of AppLocalizations
/// returned by `AppLocalizations.of(context)`.
///
/// Applications need to include `AppLocalizations.delegate()` in their app's
/// `localizationDelegates` list, and the locales they support in the app's
/// `supportedLocales` list. For example:
///
/// ```dart
/// import 'l10n/app_localizations.dart';
///
/// return MaterialApp(
///   localizationsDelegates: AppLocalizations.localizationsDelegates,
///   supportedLocales: AppLocalizations.supportedLocales,
///   home: MyApplicationHome(),
/// );
/// ```
///
/// ## Update pubspec.yaml
///
/// Please make sure to update your pubspec.yaml to include the following
/// packages:
///
/// ```yaml
/// dependencies:
///   # Internationalization support.
///   flutter_localizations:
///     sdk: flutter
///   intl: any # Use the pinned version from flutter_localizations
///
///   # Rest of dependencies
/// ```
///
/// ## iOS Applications
///
/// iOS applications define key application metadata, including supported
/// locales, in an Info.plist file that is built into the application bundle.
/// To configure the locales supported by your app, you’ll need to edit this
/// file.
///
/// First, open your project’s ios/Runner.xcworkspace Xcode workspace file.
/// Then, in the Project Navigator, open the Info.plist file under the Runner
/// project’s Runner folder.
///
/// Next, select the Information Property List item, select Add Item from the
/// Editor menu, then select Localizations from the pop-up menu.
///
/// Select and expand the newly-created Localizations item then, for each
/// locale your application supports, add a new item and select the locale
/// you wish to add from the pop-up menu in the Value field. This list should
/// be consistent with the languages listed in the AppLocalizations.supportedLocales
/// property.
abstract class AppLocalizations {
  AppLocalizations(String locale)
    : localeName = intl.Intl.canonicalizedLocale(locale.toString());

  final String localeName;

  static AppLocalizations of(BuildContext context) {
    return Localizations.of<AppLocalizations>(context, AppLocalizations)!;
  }

  static const LocalizationsDelegate<AppLocalizations> delegate =
      _AppLocalizationsDelegate();

  /// A list of this localizations delegate along with the default localizations
  /// delegates.
  ///
  /// Returns a list of localizations delegates containing this delegate along with
  /// GlobalMaterialLocalizations.delegate, GlobalCupertinoLocalizations.delegate,
  /// and GlobalWidgetsLocalizations.delegate.
  ///
  /// Additional delegates can be added by appending to this list in
  /// MaterialApp. This list does not have to be used at all if a custom list
  /// of delegates is preferred or required.
  static const List<LocalizationsDelegate<dynamic>> localizationsDelegates =
      <LocalizationsDelegate<dynamic>>[
        delegate,
        GlobalMaterialLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
      ];

  /// A list of this localizations delegate's supported locales.
  static const List<Locale> supportedLocales = <Locale>[
    Locale('cs'),
    Locale('de'),
    Locale('en'),
    Locale('es'),
    Locale('fr'),
    Locale('it'),
    Locale('ja'),
    Locale('ko'),
    Locale('pl'),
    Locale('pt'),
    Locale('ru'),
    Locale('tr'),
    Locale('zh'),
    Locale('zh', 'HK'),
    Locale('zh', 'TW'),
  ];

  /// No description provided for @cancel.
  ///
  /// In en, this message translates to:
  /// **'Cancel'**
  String get cancel;

  /// No description provided for @selectAll.
  ///
  /// In en, this message translates to:
  /// **'Select all'**
  String get selectAll;

  /// No description provided for @selectAllTrack.
  ///
  /// In en, this message translates to:
  /// **'Select this track'**
  String get selectAllTrack;

  /// No description provided for @opDuplicate.
  ///
  /// In en, this message translates to:
  /// **'Duplicate'**
  String get opDuplicate;

  /// No description provided for @opOctaveUp.
  ///
  /// In en, this message translates to:
  /// **'Octave up'**
  String get opOctaveUp;

  /// No description provided for @opOctaveDown.
  ///
  /// In en, this message translates to:
  /// **'Octave down'**
  String get opOctaveDown;

  /// No description provided for @opLengthHalf.
  ///
  /// In en, this message translates to:
  /// **'Length ÷2'**
  String get opLengthHalf;

  /// No description provided for @opLengthDouble.
  ///
  /// In en, this message translates to:
  /// **'Length ×2'**
  String get opLengthDouble;

  /// No description provided for @opFlipHorizontal.
  ///
  /// In en, this message translates to:
  /// **'Flip horizontal'**
  String get opFlipHorizontal;

  /// No description provided for @opFlipVertical.
  ///
  /// In en, this message translates to:
  /// **'Flip vertical'**
  String get opFlipVertical;

  /// No description provided for @opHumanize.
  ///
  /// In en, this message translates to:
  /// **'Humanize'**
  String get opHumanize;

  /// No description provided for @opFillGaps.
  ///
  /// In en, this message translates to:
  /// **'Fill gaps'**
  String get opFillGaps;

  /// No description provided for @opMergeNotes.
  ///
  /// In en, this message translates to:
  /// **'Merge notes'**
  String get opMergeNotes;

  /// No description provided for @opGridSplit.
  ///
  /// In en, this message translates to:
  /// **'Grid split'**
  String get opGridSplit;

  /// No description provided for @batchProcess.
  ///
  /// In en, this message translates to:
  /// **'Batch edit'**
  String get batchProcess;

  /// No description provided for @humanizeTiming.
  ///
  /// In en, this message translates to:
  /// **'Timing'**
  String get humanizeTiming;

  /// No description provided for @humanizeVelocity.
  ///
  /// In en, this message translates to:
  /// **'Velocity'**
  String get humanizeVelocity;

  /// No description provided for @humanizeGate.
  ///
  /// In en, this message translates to:
  /// **'Gate'**
  String get humanizeGate;

  /// No description provided for @batchVelocity.
  ///
  /// In en, this message translates to:
  /// **'Velocity / Value'**
  String get batchVelocity;

  /// No description provided for @batchGate.
  ///
  /// In en, this message translates to:
  /// **'Gate'**
  String get batchGate;

  /// No description provided for @batchKey.
  ///
  /// In en, this message translates to:
  /// **'Key'**
  String get batchKey;

  /// No description provided for @batchTick.
  ///
  /// In en, this message translates to:
  /// **'Tick'**
  String get batchTick;

  /// No description provided for @batchStretch.
  ///
  /// In en, this message translates to:
  /// **'Stretch'**
  String get batchStretch;

  /// No description provided for @batchLength.
  ///
  /// In en, this message translates to:
  /// **'Target length'**
  String get batchLength;

  /// No description provided for @batchLengthInvalid.
  ///
  /// In en, this message translates to:
  /// **'Invalid length'**
  String get batchLengthInvalid;

  /// No description provided for @deleteSelectionConfirm.
  ///
  /// In en, this message translates to:
  /// **'Delete selected events?'**
  String get deleteSelectionConfirm;

  /// No description provided for @save.
  ///
  /// In en, this message translates to:
  /// **'Save'**
  String get save;

  /// No description provided for @ok.
  ///
  /// In en, this message translates to:
  /// **'OK'**
  String get ok;

  /// No description provided for @back.
  ///
  /// In en, this message translates to:
  /// **'Back'**
  String get back;

  /// No description provided for @delete.
  ///
  /// In en, this message translates to:
  /// **'Delete'**
  String get delete;

  /// No description provided for @edit.
  ///
  /// In en, this message translates to:
  /// **'Edit'**
  String get edit;

  /// No description provided for @allowOverlappingNotes.
  ///
  /// In en, this message translates to:
  /// **'Allow overlapping notes'**
  String get allowOverlappingNotes;

  /// No description provided for @copy.
  ///
  /// In en, this message translates to:
  /// **'Copy'**
  String get copy;

  /// No description provided for @cut.
  ///
  /// In en, this message translates to:
  /// **'Cut'**
  String get cut;

  /// No description provided for @paste.
  ///
  /// In en, this message translates to:
  /// **'Paste'**
  String get paste;

  /// No description provided for @close.
  ///
  /// In en, this message translates to:
  /// **'Close'**
  String get close;

  /// No description provided for @add.
  ///
  /// In en, this message translates to:
  /// **'Add'**
  String get add;

  /// No description provided for @select.
  ///
  /// In en, this message translates to:
  /// **'Select'**
  String get select;

  /// No description provided for @clear.
  ///
  /// In en, this message translates to:
  /// **'Clear'**
  String get clear;

  /// No description provided for @remove.
  ///
  /// In en, this message translates to:
  /// **'Remove'**
  String get remove;

  /// No description provided for @insert.
  ///
  /// In en, this message translates to:
  /// **'Insert'**
  String get insert;

  /// No description provided for @value.
  ///
  /// In en, this message translates to:
  /// **'Value'**
  String get value;

  /// No description provided for @on.
  ///
  /// In en, this message translates to:
  /// **'On'**
  String get on;

  /// No description provided for @off.
  ///
  /// In en, this message translates to:
  /// **'Off'**
  String get off;

  /// No description provided for @loadMore.
  ///
  /// In en, this message translates to:
  /// **'Load more'**
  String get loadMore;

  /// No description provided for @notesCount.
  ///
  /// In en, this message translates to:
  /// **'{count} notes'**
  String notesCount(int count);

  /// No description provided for @openFile.
  ///
  /// In en, this message translates to:
  /// **'Open file'**
  String get openFile;

  /// No description provided for @newProject.
  ///
  /// In en, this message translates to:
  /// **'New project'**
  String get newProject;

  /// No description provided for @saveAs.
  ///
  /// In en, this message translates to:
  /// **'Save as'**
  String get saveAs;

  /// No description provided for @settings.
  ///
  /// In en, this message translates to:
  /// **'Settings'**
  String get settings;

  /// No description provided for @recentFiles.
  ///
  /// In en, this message translates to:
  /// **'Recent files'**
  String get recentFiles;

  /// No description provided for @noRecentFiles.
  ///
  /// In en, this message translates to:
  /// **'No recent files'**
  String get noRecentFiles;

  /// No description provided for @backToWorkspace.
  ///
  /// In en, this message translates to:
  /// **'Back to workspace'**
  String get backToWorkspace;

  /// No description provided for @home.
  ///
  /// In en, this message translates to:
  /// **'Home'**
  String get home;

  /// No description provided for @menu.
  ///
  /// In en, this message translates to:
  /// **'Menu'**
  String get menu;

  /// No description provided for @backToArrangement.
  ///
  /// In en, this message translates to:
  /// **'Back to arrangement'**
  String get backToArrangement;

  /// No description provided for @play.
  ///
  /// In en, this message translates to:
  /// **'Play'**
  String get play;

  /// No description provided for @pause.
  ///
  /// In en, this message translates to:
  /// **'Pause'**
  String get pause;

  /// No description provided for @stop.
  ///
  /// In en, this message translates to:
  /// **'Stop'**
  String get stop;

  /// No description provided for @quantize.
  ///
  /// In en, this message translates to:
  /// **'Quantize'**
  String get quantize;

  /// No description provided for @quantizeOff.
  ///
  /// In en, this message translates to:
  /// **'Snap off'**
  String get quantizeOff;

  /// No description provided for @quantizeAuto.
  ///
  /// In en, this message translates to:
  /// **'Auto'**
  String get quantizeAuto;

  /// No description provided for @quantizeCustom.
  ///
  /// In en, this message translates to:
  /// **'Custom…'**
  String get quantizeCustom;

  /// No description provided for @quantizeNumerator.
  ///
  /// In en, this message translates to:
  /// **'Numerator'**
  String get quantizeNumerator;

  /// No description provided for @quantizeDenominator.
  ///
  /// In en, this message translates to:
  /// **'Denominator'**
  String get quantizeDenominator;

  /// No description provided for @quantizeInvalidFraction.
  ///
  /// In en, this message translates to:
  /// **'Numerator and denominator must be positive integers.'**
  String get quantizeInvalidFraction;

  /// No description provided for @tool.
  ///
  /// In en, this message translates to:
  /// **'Tool'**
  String get tool;

  /// No description provided for @undo.
  ///
  /// In en, this message translates to:
  /// **'Undo'**
  String get undo;

  /// No description provided for @redo.
  ///
  /// In en, this message translates to:
  /// **'Redo'**
  String get redo;

  /// No description provided for @saveProject.
  ///
  /// In en, this message translates to:
  /// **'Save project'**
  String get saveProject;

  /// No description provided for @saved.
  ///
  /// In en, this message translates to:
  /// **'Saved'**
  String get saved;

  /// No description provided for @currentTrack.
  ///
  /// In en, this message translates to:
  /// **'Current track'**
  String get currentTrack;

  /// No description provided for @allTracks.
  ///
  /// In en, this message translates to:
  /// **'All tracks'**
  String get allTracks;

  /// No description provided for @project.
  ///
  /// In en, this message translates to:
  /// **'Project'**
  String get project;

  /// No description provided for @projectSettings.
  ///
  /// In en, this message translates to:
  /// **'Project settings'**
  String get projectSettings;

  /// No description provided for @songTitle.
  ///
  /// In en, this message translates to:
  /// **'Title'**
  String get songTitle;

  /// No description provided for @artist.
  ///
  /// In en, this message translates to:
  /// **'Artist'**
  String get artist;

  /// No description provided for @description.
  ///
  /// In en, this message translates to:
  /// **'Description'**
  String get description;

  /// No description provided for @ppq.
  ///
  /// In en, this message translates to:
  /// **'PPQ (rescales time when notes exist)'**
  String get ppq;

  /// No description provided for @defaultBpm.
  ///
  /// In en, this message translates to:
  /// **'Project default BPM'**
  String get defaultBpm;

  /// No description provided for @defaultTimeSig.
  ///
  /// In en, this message translates to:
  /// **'Project default time signature'**
  String get defaultTimeSig;

  /// No description provided for @bpmField.
  ///
  /// In en, this message translates to:
  /// **'BPM (0–60000000)'**
  String get bpmField;

  /// No description provided for @bpmError.
  ///
  /// In en, this message translates to:
  /// **'BPM must be 0–60000000'**
  String get bpmError;

  /// No description provided for @timeSigField.
  ///
  /// In en, this message translates to:
  /// **'Time signature (e.g. 4/4)'**
  String get timeSigField;

  /// No description provided for @soundfonts.
  ///
  /// In en, this message translates to:
  /// **'Soundfonts'**
  String get soundfonts;

  /// No description provided for @audio.
  ///
  /// In en, this message translates to:
  /// **'Audio'**
  String get audio;

  /// No description provided for @language.
  ///
  /// In en, this message translates to:
  /// **'Language'**
  String get language;

  /// No description provided for @followSystem.
  ///
  /// In en, this message translates to:
  /// **'Follow system'**
  String get followSystem;

  /// No description provided for @maxVoices.
  ///
  /// In en, this message translates to:
  /// **'Max voices'**
  String get maxVoices;

  /// No description provided for @audioBuffer.
  ///
  /// In en, this message translates to:
  /// **'Audio buffer'**
  String get audioBuffer;

  /// No description provided for @ignoreVelocity.
  ///
  /// In en, this message translates to:
  /// **'Ignore velocity'**
  String get ignoreVelocity;

  /// No description provided for @auto.
  ///
  /// In en, this message translates to:
  /// **'Auto'**
  String get auto;

  /// No description provided for @deviceDefault.
  ///
  /// In en, this message translates to:
  /// **'Device default'**
  String get deviceDefault;

  /// No description provided for @addSoundfont.
  ///
  /// In en, this message translates to:
  /// **'Add soundfont'**
  String get addSoundfont;

  /// No description provided for @frames.
  ///
  /// In en, this message translates to:
  /// **'frames'**
  String get frames;

  /// No description provided for @editTrackInfo.
  ///
  /// In en, this message translates to:
  /// **'Edit track info'**
  String get editTrackInfo;

  /// No description provided for @eventList.
  ///
  /// In en, this message translates to:
  /// **'Event list'**
  String get eventList;

  /// No description provided for @deleteTrack.
  ///
  /// In en, this message translates to:
  /// **'Delete track'**
  String get deleteTrack;

  /// No description provided for @trackName.
  ///
  /// In en, this message translates to:
  /// **'Track name'**
  String get trackName;

  /// No description provided for @songTitleField.
  ///
  /// In en, this message translates to:
  /// **'Song title'**
  String get songTitleField;

  /// No description provided for @port.
  ///
  /// In en, this message translates to:
  /// **'Port'**
  String get port;

  /// No description provided for @channel.
  ///
  /// In en, this message translates to:
  /// **'Channel'**
  String get channel;

  /// No description provided for @defaultInstrumentFollowPc.
  ///
  /// In en, this message translates to:
  /// **'Channel default instrument: follows MIDI PC'**
  String get defaultInstrumentFollowPc;

  /// No description provided for @selectChannelInstrument.
  ///
  /// In en, this message translates to:
  /// **'Select channel default instrument'**
  String get selectChannelInstrument;

  /// No description provided for @melodicBank.
  ///
  /// In en, this message translates to:
  /// **'Melodic Bank 0'**
  String get melodicBank;

  /// No description provided for @drumBank.
  ///
  /// In en, this message translates to:
  /// **'Drum Bank 128'**
  String get drumBank;

  /// No description provided for @masterTrack.
  ///
  /// In en, this message translates to:
  /// **'Edit master track'**
  String get masterTrack;

  /// No description provided for @masterTrackHint.
  ///
  /// In en, this message translates to:
  /// **'The master (Conductor) track holds tempo/time-signature events; it has no notes and no port/channel.'**
  String get masterTrackHint;

  /// No description provided for @channelInstrumentHint.
  ///
  /// In en, this message translates to:
  /// **'Channel default instrument is shared by all tracks on that channel; MIDI PC events take priority.'**
  String get channelInstrumentHint;

  /// No description provided for @mobilePortHint.
  ///
  /// In en, this message translates to:
  /// **'Only port A is audible on mobile; other ports are saved for the desktop app.'**
  String get mobilePortHint;

  /// No description provided for @tempo.
  ///
  /// In en, this message translates to:
  /// **'Tempo'**
  String get tempo;

  /// No description provided for @timeSignature.
  ///
  /// In en, this message translates to:
  /// **'Time signature'**
  String get timeSignature;

  /// No description provided for @keySignature.
  ///
  /// In en, this message translates to:
  /// **'Key signature'**
  String get keySignature;

  /// No description provided for @marker.
  ///
  /// In en, this message translates to:
  /// **'Marker'**
  String get marker;

  /// No description provided for @pitchBend.
  ///
  /// In en, this message translates to:
  /// **'Pitch Bend'**
  String get pitchBend;

  /// No description provided for @customCc.
  ///
  /// In en, this message translates to:
  /// **'Custom CC…'**
  String get customCc;

  /// No description provided for @customCcTitle.
  ///
  /// In en, this message translates to:
  /// **'Custom CC number'**
  String get customCcTitle;

  /// No description provided for @customRpnTitle.
  ///
  /// In en, this message translates to:
  /// **'Custom RPN number'**
  String get customRpnTitle;

  /// No description provided for @customRpn.
  ///
  /// In en, this message translates to:
  /// **'Custom RPN…'**
  String get customRpn;

  /// No description provided for @root.
  ///
  /// In en, this message translates to:
  /// **'Root'**
  String get root;

  /// No description provided for @scale.
  ///
  /// In en, this message translates to:
  /// **'Scale'**
  String get scale;

  /// No description provided for @major.
  ///
  /// In en, this message translates to:
  /// **'Major'**
  String get major;

  /// No description provided for @minor.
  ///
  /// In en, this message translates to:
  /// **'Natural minor'**
  String get minor;

  /// No description provided for @automationType.
  ///
  /// In en, this message translates to:
  /// **'Automation type'**
  String get automationType;

  /// No description provided for @automation.
  ///
  /// In en, this message translates to:
  /// **'Automation'**
  String get automation;

  /// No description provided for @tempoValueTitle.
  ///
  /// In en, this message translates to:
  /// **'Tempo BPM'**
  String get tempoValueTitle;

  /// No description provided for @pitchBendValueTitle.
  ///
  /// In en, this message translates to:
  /// **'Pitch Bend (0–16383, 8192 = center)'**
  String get pitchBendValueTitle;

  /// No description provided for @archivePasswordRequired.
  ///
  /// In en, this message translates to:
  /// **'Password required'**
  String get archivePasswordRequired;

  /// No description provided for @password.
  ///
  /// In en, this message translates to:
  /// **'Password'**
  String get password;

  /// No description provided for @open.
  ///
  /// In en, this message translates to:
  /// **'Open'**
  String get open;

  /// No description provided for @noMidiInArchive.
  ///
  /// In en, this message translates to:
  /// **'No MIDI files in archive'**
  String get noMidiInArchive;

  /// No description provided for @archiveOpenFailed.
  ///
  /// In en, this message translates to:
  /// **'Failed to open archive'**
  String get archiveOpenFailed;

  /// No description provided for @addTracks.
  ///
  /// In en, this message translates to:
  /// **'Add tracks'**
  String get addTracks;

  /// No description provided for @trackCount.
  ///
  /// In en, this message translates to:
  /// **'Track count (1–256)'**
  String get trackCount;

  /// No description provided for @countError.
  ///
  /// In en, this message translates to:
  /// **'Count must be 1–256'**
  String get countError;

  /// No description provided for @cycleChannels.
  ///
  /// In en, this message translates to:
  /// **'Cycle from channel'**
  String get cycleChannels;

  /// No description provided for @sameChannel.
  ///
  /// In en, this message translates to:
  /// **'Same channel for all'**
  String get sameChannel;

  /// No description provided for @startChannel.
  ///
  /// In en, this message translates to:
  /// **'Start channel'**
  String get startChannel;

  /// No description provided for @pasteSameTime.
  ///
  /// In en, this message translates to:
  /// **'Paste (same time)'**
  String get pasteSameTime;

  /// No description provided for @goToLocation.
  ///
  /// In en, this message translates to:
  /// **'Go to location'**
  String get goToLocation;

  /// No description provided for @eventCopied.
  ///
  /// In en, this message translates to:
  /// **'Event copied'**
  String get eventCopied;

  /// No description provided for @noEvents.
  ///
  /// In en, this message translates to:
  /// **'No events'**
  String get noEvents;

  /// No description provided for @velocity.
  ///
  /// In en, this message translates to:
  /// **'Velocity'**
  String get velocity;

  /// No description provided for @length.
  ///
  /// In en, this message translates to:
  /// **'Length'**
  String get length;

  /// No description provided for @velocityRange.
  ///
  /// In en, this message translates to:
  /// **'Velocity (1–127)'**
  String get velocityRange;

  /// No description provided for @lengthTicks.
  ///
  /// In en, this message translates to:
  /// **'Length (ticks)'**
  String get lengthTicks;

  /// No description provided for @editNote.
  ///
  /// In en, this message translates to:
  /// **'Edit note'**
  String get editNote;

  /// No description provided for @sortHint.
  ///
  /// In en, this message translates to:
  /// **'Time → pitch/value → length'**
  String get sortHint;

  /// No description provided for @noteLabel.
  ///
  /// In en, this message translates to:
  /// **'Note'**
  String get noteLabel;

  /// No description provided for @markerLabel.
  ///
  /// In en, this message translates to:
  /// **'Marker'**
  String get markerLabel;

  /// No description provided for @openFirst.
  ///
  /// In en, this message translates to:
  /// **'Open a MIDI or create a project first'**
  String get openFirst;

  /// No description provided for @exportFailed.
  ///
  /// In en, this message translates to:
  /// **'Project export failed'**
  String get exportFailed;

  /// No description provided for @copied.
  ///
  /// In en, this message translates to:
  /// **'Copied'**
  String get copied;

  /// No description provided for @toolSelect.
  ///
  /// In en, this message translates to:
  /// **'Select'**
  String get toolSelect;

  /// No description provided for @toolHand.
  ///
  /// In en, this message translates to:
  /// **'Hand'**
  String get toolHand;

  /// No description provided for @toolPencil.
  ///
  /// In en, this message translates to:
  /// **'Pencil'**
  String get toolPencil;

  /// No description provided for @toolEraser.
  ///
  /// In en, this message translates to:
  /// **'Eraser'**
  String get toolEraser;

  /// No description provided for @toolBrush.
  ///
  /// In en, this message translates to:
  /// **'Brush'**
  String get toolBrush;

  /// No description provided for @markerRename.
  ///
  /// In en, this message translates to:
  /// **'Rename marker'**
  String get markerRename;

  /// No description provided for @markerDelete.
  ///
  /// In en, this message translates to:
  /// **'Delete marker'**
  String get markerDelete;

  /// No description provided for @markerText.
  ///
  /// In en, this message translates to:
  /// **'Marker text'**
  String get markerText;

  /// No description provided for @markerTextOptional.
  ///
  /// In en, this message translates to:
  /// **'Marker text (optional)'**
  String get markerTextOptional;

  /// No description provided for @untitled.
  ///
  /// In en, this message translates to:
  /// **'Untitled'**
  String get untitled;

  /// No description provided for @ppqError.
  ///
  /// In en, this message translates to:
  /// **'PPQ must be an integer 1–32767'**
  String get ppqError;

  /// No description provided for @channelInstrumentValue.
  ///
  /// In en, this message translates to:
  /// **'Channel default instrument: {name} (Bank {bank} / Program {program})'**
  String channelInstrumentValue(String name, int bank, int program);

  /// No description provided for @eventNoteTitle.
  ///
  /// In en, this message translates to:
  /// **'Note {name}'**
  String eventNoteTitle(String name);

  /// No description provided for @eventNoteDetail.
  ///
  /// In en, this message translates to:
  /// **'{time} · {track} · vel {velocity} · len {length}'**
  String eventNoteDetail(String time, String track, int velocity, int length);

  /// No description provided for @eventValueDetail.
  ///
  /// In en, this message translates to:
  /// **'{time} · {track} · {value} (0–{max})'**
  String eventValueDetail(String time, String track, int value, int max);

  /// No description provided for @centerHint.
  ///
  /// In en, this message translates to:
  /// **'8192 = center'**
  String get centerHint;

  /// No description provided for @jumpTo.
  ///
  /// In en, this message translates to:
  /// **'Jump'**
  String get jumpTo;

  /// No description provided for @exportAudio.
  ///
  /// In en, this message translates to:
  /// **'Export audio'**
  String get exportAudio;

  /// No description provided for @exportMidi.
  ///
  /// In en, this message translates to:
  /// **'Export MIDI'**
  String get exportMidi;

  /// No description provided for @exporting.
  ///
  /// In en, this message translates to:
  /// **'Exporting…'**
  String get exporting;

  /// No description provided for @exportDone.
  ///
  /// In en, this message translates to:
  /// **'Export complete'**
  String get exportDone;

  /// No description provided for @bitDepth.
  ///
  /// In en, this message translates to:
  /// **'Bit depth'**
  String get bitDepth;

  /// No description provided for @cancelling.
  ///
  /// In en, this message translates to:
  /// **'Cancelling…'**
  String get cancelling;

  /// No description provided for @exportFailedShort.
  ///
  /// In en, this message translates to:
  /// **'Export failed'**
  String get exportFailedShort;

  /// No description provided for @prevPage.
  ///
  /// In en, this message translates to:
  /// **'Previous page'**
  String get prevPage;

  /// No description provided for @nextPage.
  ///
  /// In en, this message translates to:
  /// **'Next page'**
  String get nextPage;

  /// No description provided for @importingSoundfont.
  ///
  /// In en, this message translates to:
  /// **'Importing soundfont…'**
  String get importingSoundfont;

  /// No description provided for @importSoundfontDone.
  ///
  /// In en, this message translates to:
  /// **'Soundfont imported'**
  String get importSoundfontDone;

  /// No description provided for @noSoundfontInArchive.
  ///
  /// In en, this message translates to:
  /// **'No soundfonts in archive'**
  String get noSoundfontInArchive;

  /// No description provided for @loadingFile.
  ///
  /// In en, this message translates to:
  /// **'Loading…'**
  String get loadingFile;

  /// No description provided for @loadFailed.
  ///
  /// In en, this message translates to:
  /// **'Load failed'**
  String get loadFailed;

  /// No description provided for @savingProject.
  ///
  /// In en, this message translates to:
  /// **'Saving project…'**
  String get savingProject;

  /// No description provided for @saveFailed.
  ///
  /// In en, this message translates to:
  /// **'Save failed'**
  String get saveFailed;

  /// No description provided for @unsavedChanges.
  ///
  /// In en, this message translates to:
  /// **'Unsaved changes'**
  String get unsavedChanges;

  /// No description provided for @unsavedChangesPrompt.
  ///
  /// In en, this message translates to:
  /// **'Continuing will discard the current changes. Save first?'**
  String get unsavedChangesPrompt;

  /// No description provided for @dontSave.
  ///
  /// In en, this message translates to:
  /// **'Don\'t save'**
  String get dontSave;
}

class _AppLocalizationsDelegate
    extends LocalizationsDelegate<AppLocalizations> {
  const _AppLocalizationsDelegate();

  @override
  Future<AppLocalizations> load(Locale locale) {
    return SynchronousFuture<AppLocalizations>(lookupAppLocalizations(locale));
  }

  @override
  bool isSupported(Locale locale) => <String>[
    'cs',
    'de',
    'en',
    'es',
    'fr',
    'it',
    'ja',
    'ko',
    'pl',
    'pt',
    'ru',
    'tr',
    'zh',
  ].contains(locale.languageCode);

  @override
  bool shouldReload(_AppLocalizationsDelegate old) => false;
}

AppLocalizations lookupAppLocalizations(Locale locale) {
  // Lookup logic when language+country codes are specified.
  switch (locale.languageCode) {
    case 'zh':
      {
        switch (locale.countryCode) {
          case 'HK':
            return AppLocalizationsZhHk();
          case 'TW':
            return AppLocalizationsZhTw();
        }
        break;
      }
  }

  // Lookup logic when only language code is specified.
  switch (locale.languageCode) {
    case 'cs':
      return AppLocalizationsCs();
    case 'de':
      return AppLocalizationsDe();
    case 'en':
      return AppLocalizationsEn();
    case 'es':
      return AppLocalizationsEs();
    case 'fr':
      return AppLocalizationsFr();
    case 'it':
      return AppLocalizationsIt();
    case 'ja':
      return AppLocalizationsJa();
    case 'ko':
      return AppLocalizationsKo();
    case 'pl':
      return AppLocalizationsPl();
    case 'pt':
      return AppLocalizationsPt();
    case 'ru':
      return AppLocalizationsRu();
    case 'tr':
      return AppLocalizationsTr();
    case 'zh':
      return AppLocalizationsZh();
  }

  throw FlutterError(
    'AppLocalizations.delegate failed to load unsupported locale "$locale". This is likely '
    'an issue with the localizations generation tool. Please file an issue '
    'on GitHub with a reproducible sample app and the gen-l10n configuration '
    'that was used.',
  );
}
