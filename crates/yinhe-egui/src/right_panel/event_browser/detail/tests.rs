use super::super::table::EVENT_PAGE_SIZE;
use super::summary::summarize_targets;
use super::*;
use yinhe_types::automation::{AutomationEvent, ParamDevice, xsynth_param};
use yinhe_types::{AutomationLane, SegmentShape};

fn lane(target: AutomationTarget, count: usize) -> AutomationLane {
    AutomationLane {
        target,
        track: 0,
        events: (0..count)
            .map(|i| AutomationEvent {
                id: i as u32,
                tick: i as u32 * 10,
                value: 0.5,
                shape: SegmentShape::Step,
            })
            .collect(),
    }
}

#[test]
fn total_pages_rounds_up_and_is_at_least_one() {
    assert_eq!(total_pages(0), 1);
    assert_eq!(total_pages(1), 1);
    assert_eq!(total_pages(EVENT_PAGE_SIZE), 1);
    assert_eq!(total_pages(EVENT_PAGE_SIZE + 1), 2);
}

#[test]
fn paginate_clamps_out_of_range_page() {
    let mut state = EventBrowserState::default();
    let items: Vec<u32> = (0..(EVENT_PAGE_SIZE * 3)).map(|i| i as u32).collect();
    state.event_page = 99;
    let (page, start, slice) = paginate(&mut state, &items);
    assert_eq!(page, 2);
    assert_eq!(state.event_page, 2);
    assert_eq!(start, EVENT_PAGE_SIZE * 2);
    assert_eq!(slice.len(), EVENT_PAGE_SIZE);
    assert_eq!(slice[0], (EVENT_PAGE_SIZE * 2) as u32);
}

#[test]
fn paginate_slices_tail_page() {
    let mut state = EventBrowserState::default();
    let items: Vec<u32> = (0..(EVENT_PAGE_SIZE + 7)).map(|i| i as u32).collect();
    state.event_page = 1;
    let (page, start, slice) = paginate(&mut state, &items);
    assert_eq!(page, 1);
    assert_eq!(start, EVENT_PAGE_SIZE);
    assert_eq!(slice.len(), 7);
    assert_eq!(slice[6], (EVENT_PAGE_SIZE + 6) as u32);
}

#[test]
fn paginate_empty_list_is_first_page() {
    let mut state = EventBrowserState::default();
    let items: Vec<u32> = Vec::new();
    let (page, start, slice) = paginate(&mut state, &items);
    assert_eq!(page, 0);
    assert_eq!(start, 0);
    assert!(slice.is_empty());
}

#[test]
fn summarize_targets_classifies_and_preserves_cc_order() {
    let lanes = vec![
        lane(AutomationTarget::CC { controller: 1 }, 2),
        lane(AutomationTarget::CC { controller: 7 }, 3),
        lane(AutomationTarget::CC { controller: 1 }, 4),
        lane(
            AutomationTarget::Param {
                device: ParamDevice::ChannelInstrument { channel: 0 },
                id: xsynth_param::PITCH_BEND,
                name: String::new(),
            },
            5,
        ),
        lane(
            AutomationTarget::Param {
                device: ParamDevice::ChannelInstrument { channel: 0 },
                id: 42,
                name: "Cutoff".into(),
            },
            6,
        ),
        lane(AutomationTarget::Rpn { parameter: 0 }, 7),
        lane(AutomationTarget::Nrpn { parameter: 1 }, 8),
        lane(AutomationTarget::Tempo, 9),
    ];
    let s = summarize_targets(&lanes);
    assert_eq!(s.cc_total, 9);
    assert_eq!(s.cc_per_controller, vec![(1, 6), (7, 3)]);
    assert_eq!(s.pb_total, 5);
    assert_eq!(s.param_total, 6);
    assert_eq!(s.rpn_total, 15);
}

#[test]
fn target_summary_merge_accumulates_across_tracks() {
    let mut total = summarize_targets(&[lane(AutomationTarget::CC { controller: 1 }, 2)]);
    total.merge(summarize_targets(&[
        lane(AutomationTarget::CC { controller: 1 }, 3),
        lane(AutomationTarget::CC { controller: 7 }, 4),
    ]));
    assert_eq!(total.cc_total, 9);
    assert_eq!(total.cc_per_controller, vec![(1, 5), (7, 4)]);
}
