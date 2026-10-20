// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Turkish (`tr`).
class AppLocalizationsTr extends AppLocalizations {
  AppLocalizationsTr([String locale = 'tr']) : super(locale);

  @override
  String get cancel => 'İptal';

  @override
  String get save => 'Kaydet';

  @override
  String get ok => 'Tamam';

  @override
  String get back => 'Geri';

  @override
  String get delete => 'Sil';

  @override
  String get edit => 'Düzen';

  @override
  String get copy => 'Copy';

  @override
  String get paste => 'Yapıştır';

  @override
  String get close => 'Kapat';

  @override
  String get add => 'Ekle';

  @override
  String get select => 'Seç';

  @override
  String get clear => 'Temizle';

  @override
  String get remove => 'Kaldır';

  @override
  String get insert => 'Ekle';

  @override
  String get value => 'Değer';

  @override
  String get on => 'On';

  @override
  String get off => 'Off';

  @override
  String get loadMore => 'Load more';

  @override
  String notesCount(int count) {
    return '$count nota';
  }

  @override
  String get openFile => 'Dosya Aç…';

  @override
  String get newProject => 'Yeni Proje';

  @override
  String get saveAs => 'Farklı Kaydet';

  @override
  String get settings => 'Ayarlar';

  @override
  String get recentFiles => 'Son Değiştirilenler';

  @override
  String get noRecentFiles => 'Son dosya yok';

  @override
  String get backToWorkspace => 'Çalışma alanına dön';

  @override
  String get home => 'Ana sayfa';

  @override
  String get backToArrangement => 'Düzenlemeye dön';

  @override
  String get play => 'Çalma';

  @override
  String get pause => 'Duraklat';

  @override
  String get stop => 'Durdur';

  @override
  String get quantize => 'Niceleme';

  @override
  String get quantizeOff => 'Kapalı (hizalama yok)';

  @override
  String get quantizeAuto => 'Otomatik (uyarlanabilir)';

  @override
  String get tool => 'Araçlar';

  @override
  String get undo => 'Geri Al';

  @override
  String get redo => 'Yinele';

  @override
  String get saveProject => 'Kaydet';

  @override
  String get saved => 'Proje Kaydedildi';

  @override
  String get currentTrack => 'Geçerli parça';

  @override
  String get allTracks => 'Tümünü Göster';

  @override
  String get project => 'Proje';

  @override
  String get projectSettings => 'Proje Ayarları';

  @override
  String get songTitle => 'Proje Adı';

  @override
  String get artist => 'Sanatçı';

  @override
  String get description => 'Açıklama';

  @override
  String get ppq =>
      'Çeyrek nota başına vuruş; notalar varken değiştirmek yeniden ölçekleme ister';

  @override
  String get defaultBpm => 'Proje varsayılan BPM';

  @override
  String get defaultTimeSig => 'Proje varsayılan ölçü';

  @override
  String get bpmField => 'BPM (0–60000000)';

  @override
  String get bpmError => 'BPM must be 0–60000000';

  @override
  String get timeSigField => 'Zaman Ölçüsü…';

  @override
  String get soundfonts => 'SoundFont';

  @override
  String get audio => 'Ses';

  @override
  String get language => 'Dil';

  @override
  String get followSystem => 'Sistemi takip et';

  @override
  String get maxVoices => 'Maksimum ses';

  @override
  String get audioBuffer => 'Tampon Boyutu';

  @override
  String get ignoreVelocity => 'Vuruş hızını yoksay';

  @override
  String get auto => 'Otomatik';

  @override
  String get deviceDefault => 'Cihaz varsayılanı';

  @override
  String get addSoundfont => 'SoundFont Ekle';

  @override
  String get frames => 'kare';

  @override
  String get editTrackInfo => 'Düzen';

  @override
  String get eventList => 'Olaylar';

  @override
  String get deleteTrack => 'Sil';

  @override
  String get trackName => 'Parça Adı:';

  @override
  String get songTitleField => 'Şarkı Adı:';

  @override
  String get port => 'Bağlantı Noktası';

  @override
  String get channel => 'Kanal';

  @override
  String get defaultInstrumentFollowPc =>
      'Channel default instrument: follows MIDI PC';

  @override
  String get selectChannelInstrument => 'Seç';

  @override
  String get melodicBank => 'Melodic Bank 0';

  @override
  String get drumBank => 'Drum Bank 128';

  @override
  String get masterTrack => 'Düzen';

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
  String get timeSignature => 'Zaman Ölçüsü…';

  @override
  String get keySignature => 'Ton / Dizi…';

  @override
  String get marker => 'İşaret ekle';

  @override
  String get pitchBend => 'Pitch Bend';

  @override
  String get customCc => 'Özel CC';

  @override
  String get customCcTitle => 'Custom CC number';

  @override
  String get customRpnTitle => 'Custom RPN number';

  @override
  String get customRpn => 'Özel RPN';

  @override
  String get root => 'Kök';

  @override
  String get scale => 'Key Signature';

  @override
  String get major => 'Majör';

  @override
  String get minor => 'Doğal minör';

  @override
  String get automationType => 'Otomasyon…';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = center)';

  @override
  String get archivePasswordRequired => 'Parola gerekli';

  @override
  String get password => 'Parolayı girin';

  @override
  String get open => 'Aç';

  @override
  String get noMidiInArchive => 'No MIDI files in archive';

  @override
  String get archiveOpenFailed => 'Failed to open archive';

  @override
  String get addTracks => 'Parça ekle';

  @override
  String get trackCount => 'Sayı';

  @override
  String get countError => 'Count must be 1–256';

  @override
  String get cycleChannels => 'Cycle from channel';

  @override
  String get sameChannel => 'Same channel for all';

  @override
  String get startChannel => 'Start channel';

  @override
  String get pasteSameTime => 'Yapıştır';

  @override
  String get goToLocation => 'Go to location';

  @override
  String get eventCopied => 'Event copied';

  @override
  String get noEvents => 'No events';

  @override
  String get velocity => 'Vuruş Hızı';

  @override
  String get length => 'Gate';

  @override
  String get velocityRange => 'Vuruş Hızı';

  @override
  String get lengthTicks => 'Gate';

  @override
  String get editNote => 'Notalar';

  @override
  String get sortHint => 'Time → pitch/value → length';

  @override
  String get noteLabel => 'Notalar';

  @override
  String get markerLabel => 'İşaret ekle';

  @override
  String get openFirst => 'Open a MIDI or create a project first';

  @override
  String get exportFailed => 'Dışa Aktarma Başarısız';

  @override
  String get copied => 'Copied';

  @override
  String get toolSelect => 'Seç';

  @override
  String get toolHand => 'Kaydır';

  @override
  String get toolPencil => 'Kalem';

  @override
  String get toolEraser => 'Silgi';

  @override
  String get toolBrush => 'Fırça';
}
