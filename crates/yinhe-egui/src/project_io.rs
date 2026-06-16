use yinhe_project::*;

/// Convert a MidiFile into a ProjectArchive.
pub fn midi_to_archive(midi: &yinhe_midi::MidiFile) -> ProjectArchive {
    yinhe_project::conversion::midi_to_archive(midi)
}

/// Same as `midi_to_archive` but uses caller-provided track names.
pub fn midi_to_archive_with_names(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
    progress: Option<&dyn Fn(f32, &str)>,
) -> ProjectArchive {
    yinhe_project::conversion::midi_to_archive_with_names(midi, track_names, progress)
}

/// Convert a ProjectArchive back into a MidiFile.
pub fn archive_to_midi(archive: &ProjectArchive) -> yinhe_midi::MidiFile {
    yinhe_project::conversion::archive_to_midi(archive)
}

/// Save a document as a .yin file.
pub fn save_project(
    doc: &crate::document::Document,
    path: &str,
    global_enabled: bool,
) -> std::io::Result<()> {
    let archive = build_archive(doc, global_enabled);
    archive.write_to(path)
}

/// Build a ProjectArchive from a Document (without writing to disk).
pub fn build_archive(
    doc: &crate::document::Document,
    global_enabled: bool,
) -> ProjectArchive {
    let sf_overrides: Vec<(u8, Vec<SfEntryJson>)> = doc
        .edit
        .project_sf
        .overrides
        .iter()
        .map(|(port, entries)| {
            (
                *port,
                entries
                    .iter()
                    .map(|e| SfEntryJson {
                        path: e.path.clone(),
                        name: e.name.clone(),
                        enabled: e.enabled,
                    })
                    .collect(),
            )
        })
        .collect();

    yinhe_project::conversion::build_archive_from(
        doc.midi(),
        doc.track_names(),
        &doc.data.project_name,
        &doc.data.project_artist,
        doc.data.project_ppq,
        doc.data.compression_level,
        &doc.data.project_description,
        &sf_overrides,
        global_enabled,
    )
}

/// Build a ProjectArchive from raw fields (usable from a background thread).
pub fn build_archive_from(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
    project_name: &str,
    project_artist: &str,
    project_ppq: u32,
    compression_level: i32,
    project_description: &str,
    project_sf_overrides: &[(u8, Vec<SfEntryJson>)],
    global_enabled: bool,
) -> ProjectArchive {
    yinhe_project::conversion::build_archive_from(
        midi,
        track_names,
        project_name,
        project_artist,
        project_ppq,
        compression_level,
        project_description,
        project_sf_overrides,
        global_enabled,
    )
}

/// Load a .yin file and return a MidiFile + file stem name + the archive.
pub fn load_project_full(path: &str) -> std::io::Result<(yinhe_midi::MidiFile, String, ProjectArchive)> {
    yinhe_project::conversion::load_project_full(path)
}

/// Load a .yin file and return a MidiFile + file stem name.
pub fn load_project(path: &str) -> std::io::Result<(yinhe_midi::MidiFile, String)> {
    yinhe_project::conversion::load_project(path)
}

/// Export the current document as a standard MIDI file.
pub fn export_midi(doc: &crate::document::Document, path: &str) -> Result<(), String> {
    yinhe_project::conversion::export_midi(doc.midi(), doc.track_names(), path)
}
