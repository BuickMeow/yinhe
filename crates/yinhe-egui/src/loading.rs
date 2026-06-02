use std::sync::mpsc;

use yinhe_midi::LoadProgress;

/// Events sent from the background loading thread to the UI thread.
pub(crate) enum MidiLoadEvent {
    Progress(LoadProgress),
    Complete(Box<Result<yinhe_midi::MidiFile, yinhe_midi::MidiError>>),
}

/// Tracks the state of an in-flight MIDI load operation.
pub(crate) struct MidiLoader {
    pub path: String,
    pub rx: mpsc::Receiver<MidiLoadEvent>,
    pub current_progress: Option<LoadProgress>,
}
