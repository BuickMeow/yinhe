//! 复现：工程保存后无法二次打开的排查测试。

use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_test_helpers::*;

fn save_and_reopen(doc: &mut Document, path: &str) -> Result<Document, String> {
    // 模拟 App::save_project_async 的三步同步
    doc.sync_overrides_to_model();
    doc.data.sync_project_file();
    doc.data.sync_mapping_file();

    let model = doc.data.model.clone();
    let project_file = doc.data.project_file.clone();
    let mapping_file = doc.data.mapping_file.clone();
    yinhe_yin::save_yin_with_files(&model, path, &project_file, &mapping_file)
        .map_err(|e| format!("save failed: {e}"))?;

    // 模拟 FileLoader::start_yin + poll
    let (model2, sf, mapping2) =
        yinhe_yin::load_yin_with_sf(path).map_err(|e| format!("load failed: {e}"))?;
    let project_file2 =
        yinhe_yin::ProjectFile::from_meta_with_sf(&model2.meta, sf.mode, sf.overrides.clone());
    let doc2 = Document::from_model(
        path,
        model2,
        QuantizePreset::Fraction(1, 4),
        QuantizePreset::Fraction(1, 16),
        project_file2,
        mapping2,
    )
    .map_err(|e| format!("from_model failed: {e}"))?;
    Ok(doc2)
}

#[test]
fn document_save_and_reopen_roundtrip() {
    let mut doc = make_test_document();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.yin");
    let path_str = path.to_string_lossy().to_string();

    let notes_before: Vec<_> = doc
        .data
        .model
        .notes
        .iter()
        .enumerate()
        .flat_map(|(k, bucket)| {
            bucket
                .iter()
                .map(move |n| (n.track, n.start_tick, n.end_tick, k as u8, n.velocity))
                .collect::<Vec<_>>()
        })
        .collect();

    let doc2 = save_and_reopen(&mut doc, &path_str).expect("save→load→from_model 必须成功");

    let notes_after: Vec<_> = doc2
        .data
        .model
        .notes
        .iter()
        .enumerate()
        .flat_map(|(k, bucket)| {
            bucket
                .iter()
                .map(move |n| (n.track, n.start_tick, n.end_tick, k as u8, n.velocity))
                .collect::<Vec<_>>()
        })
        .collect();

    assert_eq!(
        notes_after, notes_before,
        "音符数据在保存→重开后必须逐条一致"
    );
    assert_eq!(
        doc2.data.model.tracks.len(),
        doc.data.model.tracks.len(),
        "轨道数必须一致"
    );
}

/// 解析真实 MIDI → 建文档 → 保存 → 重开（模拟用户实际流程）
#[test]
fn real_midi_save_and_reopen_roundtrip() {
    let midi_dir = "/Users/jieneng/Music/MIDIs";
    let files = [
        "352_BPM81.mid",
        "A Tale Of Sea Dragons BMver (Future D Concert Only).mid",
        "5K 5,555,555 notes by The Atom Bomb.mid",
    ];
    for f in files {
        let midi_path = format!("{midi_dir}/{f}");
        if !std::path::Path::new(&midi_path).exists() {
            eprintln!("跳过不存在的文件: {midi_path}");
            continue;
        }
        let data = std::fs::read(&midi_path).unwrap();
        let model = yinhe_mid2::parse_bytes_with_encoding(
            &data,
            yinhe_mid2::MidiImportEncoding::Utf8,
            |_| {},
        )
        .expect("MIDI 解析失败");
        let mut doc = Document::from_model(
            &midi_path,
            model,
            QuantizePreset::Fraction(1, 4),
            QuantizePreset::Fraction(1, 16),
            Default::default(),
            Default::default(),
        )
        .expect("from_model 失败");

        let note_count = doc.data.model.note_count;
        let dir = tempfile::tempdir().unwrap();
        let yin_path = dir.path().join(format!("{f}.yin"));
        let yin_str = yin_path.to_string_lossy().to_string();

        let doc2 = save_and_reopen(&mut doc, &yin_str).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(
            doc2.data.model.note_count, note_count,
            "{f}: 重开后音符数不一致"
        );
        eprintln!("{f}: {note_count} notes 保存→重开 OK");
    }
}
