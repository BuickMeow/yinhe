use yinhe_types::TimeSigEvent;

/// Global tempo event, sorted by tick.
#[derive(Clone, Debug)]
pub(crate) struct TempoEvent {
    pub tick: u32,
    pub micros_per_quarter: u64,
}

/// Collect all tempo change events from all tracks.
pub(crate) fn collect_tempo_events(tracks: &[midly::Track]) -> Vec<TempoEvent> {
    let mut events = Vec::new();
    for track in tracks {
        let mut tick: u32 = 0;
        for event in track {
            tick += event.delta.as_int();
            if let midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(us)) = event.kind {
                events.push(TempoEvent {
                    tick,
                    micros_per_quarter: us.as_int() as u64,
                });
            }
        }
    }
    events.sort_by_key(|e| e.tick);
    events.dedup_by_key(|e| e.tick);
    events
}

/// Collect all time signature events from all tracks.
pub(crate) fn collect_time_sig_events(tracks: &[midly::Track]) -> Vec<TimeSigEvent> {
    let mut events = Vec::new();
    for track in tracks {
        let mut tick: u32 = 0;
        for event in track {
            tick += event.delta.as_int();
            if let midly::TrackEventKind::Meta(midly::MetaMessage::TimeSignature(
                numerator,
                denominator,
                _,
                _,
            )) = event.kind
            {
                events.push(TimeSigEvent {
                    tick,
                    numerator,
                    denominator,
                });
            }
        }
    }
    events.sort_by_key(|e| e.tick);
    events.dedup_by_key(|e| e.tick);
    events
}
