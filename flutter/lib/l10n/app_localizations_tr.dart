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
  String get selectAll => 'Tümünü seç';

  @override
  String get selectAllTrack => 'Bu kanalı seç';

  @override
  String get opDuplicate => 'Çoğalt';

  @override
  String get opOctaveUp => 'Oktav yukarı';

  @override
  String get opOctaveDown => 'Oktav aşağı';

  @override
  String get opLengthHalf => 'Süre ÷2';

  @override
  String get opLengthDouble => 'Süre ×2';

  @override
  String get opFlipHorizontal => 'Yatay çevir';

  @override
  String get opFlipVertical => 'Dikey çevir';

  @override
  String get opHumanize => 'İnsancıllaştır';

  @override
  String get opFillGaps => 'Boşlukları doldur';

  @override
  String get opMergeNotes => 'Notaları birleştir';

  @override
  String get opGridSplit => 'Izgaraya göre böl';

  @override
  String get batchProcess => 'Toplu düzenleme';

  @override
  String get humanizeTiming => 'Zamanlama';

  @override
  String get humanizeVelocity => 'Hız';

  @override
  String get humanizeGate => 'Süre';

  @override
  String get batchVelocity => 'Hız / Değer';

  @override
  String get batchGate => 'Süre';

  @override
  String get batchKey => 'Perde';

  @override
  String get batchTick => 'Konum';

  @override
  String get batchStretch => 'Uzatma';

  @override
  String get batchLength => 'Hedef uzunluk';

  @override
  String get batchLengthInvalid => 'Geçersiz uzunluk';

  @override
  String get deleteSelectionConfirm => 'Seçili olaylar silinsin mi?';

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
  String get allowOverlappingNotes => 'Notların üst üste gelmesine izin ver';

  @override
  String get copy => 'Kopyala';

  @override
  String get cut => 'Kes';

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
  String get on => 'Açık';

  @override
  String get off => 'Kapalı';

  @override
  String get loadMore => 'Daha fazla yükle';

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
  String get menu => 'Menü';

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
  String get quantizeAuto => 'Otomatik';

  @override
  String get quantizeCustom => 'Özel…';

  @override
  String get quantizeNumerator => 'Pay';

  @override
  String get quantizeDenominator => 'Payda';

  @override
  String get quantizeInvalidFraction =>
      'Pay ve payda pozitif tam sayı olmalıdır.';

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
  String get bpmError => 'BPM 0–60000000 arasında olmalı';

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
      'Bu kanalın varsayılan enstrümanı: MIDI PC\'yi izler';

  @override
  String get selectChannelInstrument => 'Seç';

  @override
  String get melodicBank => 'Melodik Bank 0';

  @override
  String get drumBank => 'Davul Bank 128';

  @override
  String get masterTrack => 'Düzen';

  @override
  String get masterTrackHint =>
      'Master (Conductor) izi tempo/ölçü gibi genel olayları taşır; notası ve port/kanal ayarı yoktur.';

  @override
  String get channelInstrumentHint =>
      'Kanalın varsayılan enstrümanı: o kanaldaki tüm izler paylaşır; MIDI PC olayları önceliklidir.';

  @override
  String get mobilePortHint =>
      'Mobilde yalnızca A portu duyulur; diğer portlar masaüstü uygulaması için kaydedilir.';

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
  String get customCcTitle => 'Özel CC numarası';

  @override
  String get customRpnTitle => 'Özel RPN numarası';

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
  String get automation => 'Otomasyon';

  @override
  String get tempoValueTitle => 'Tempo BPM';

  @override
  String get pitchBendValueTitle => 'Pitch Bend (0–16383, 8192 = merkez)';

  @override
  String get archivePasswordRequired => 'Parola gerekli';

  @override
  String get password => 'Parolayı girin';

  @override
  String get open => 'Aç';

  @override
  String get noMidiInArchive => 'Arşivde MIDI dosyası yok';

  @override
  String get archiveOpenFailed => 'Arşiv açılamadı';

  @override
  String get addTracks => 'Parça ekle';

  @override
  String get trackCount => 'Sayı';

  @override
  String get countError => 'Adet 1–256 olmalı';

  @override
  String get cycleChannels => 'Kanaldan itibaren döngü';

  @override
  String get sameChannel => 'Tümü aynı kanal';

  @override
  String get startChannel => 'Başlangıç kanalı';

  @override
  String get pasteSameTime => 'Yapıştır';

  @override
  String get goToLocation => 'Konuma git';

  @override
  String get eventCopied => 'Olay kopyalandı';

  @override
  String get noEvents => 'Olay yok';

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
  String get sortHint => 'Zaman → perde/değer → uzunluk';

  @override
  String get noteLabel => 'Notalar';

  @override
  String get markerLabel => 'İşaret ekle';

  @override
  String get openFirst => 'Önce bir MIDI açın veya proje oluşturun';

  @override
  String get exportFailed => 'Dışa Aktarma Başarısız';

  @override
  String get copied => 'Kopyalandı';

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

  @override
  String get markerRename => 'İşaretçiyi yeniden adlandır';

  @override
  String get markerDelete => 'İşaretçiyi sil';

  @override
  String get markerText => 'İşaretçi metni';

  @override
  String get markerTextOptional => 'İşaretçi metni (isteğe bağlı)';

  @override
  String get untitled => 'Adsız';

  @override
  String get ppqError => 'PPQ 1–32767 arasında bir tam sayı olmalı';

  @override
  String channelInstrumentValue(String name, int bank, int program) {
    return 'Kanalın varsayılan enstrümanı: $name (Bank $bank / Program $program)';
  }

  @override
  String eventNoteTitle(String name) {
    return 'Nota $name';
  }

  @override
  String eventNoteDetail(String time, String track, int velocity, int length) {
    return '$time · $track · hız $velocity · uzunluk $length';
  }

  @override
  String eventValueDetail(String time, String track, int value, int max) {
    return '$time · $track · $value (0–$max)';
  }

  @override
  String get centerHint => '8192 = merkez';

  @override
  String get jumpTo => 'Git';

  @override
  String get exportAudio => 'Ses dışa aktar';

  @override
  String get exportMidi => 'MIDI dışa aktar';

  @override
  String get exporting => 'Dışa aktarılıyor…';

  @override
  String get exportDone => 'Dışa aktarma tamamlandı';

  @override
  String get bitDepth => 'Bit derinliği';

  @override
  String get cancelling => 'İptal ediliyor…';

  @override
  String get exportFailedShort => 'Dışa aktarma başarısız';

  @override
  String get prevPage => 'Önceki sayfa';

  @override
  String get nextPage => 'Sonraki sayfa';

  @override
  String get importingSoundfont => 'Soundfont içe aktarılıyor…';

  @override
  String get importSoundfontDone => 'Soundfont içe aktarıldı';

  @override
  String get noSoundfontInArchive => 'Arşivde soundfont yok';

  @override
  String get loadingFile => 'Yükleniyor…';

  @override
  String get loadFailed => 'Yükleme başarısız';

  @override
  String get savingProject => 'Proje kaydediliyor…';

  @override
  String get saveFailed => 'Kaydetme başarısız';

  @override
  String get unsavedChanges => 'Kaydedilmemiş değişiklikler';

  @override
  String get unsavedChangesPrompt =>
      'Devam etmek mevcut değişiklikleri siler. Önce kaydedilsin mi?';

  @override
  String get dontSave => 'Kaydetme';
}
