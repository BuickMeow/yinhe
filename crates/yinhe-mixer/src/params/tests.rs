use super::*;

#[test]
fn default_has_256_racks_and_no_bulk_channels() {
    let mut p = MixerParams::default();
    p.ensure_len();
    assert_eq!(p.racks.len(), CHANNEL_COUNT);
    // 方案 C：不再预建 256 条共享 MIDI LC；只有 AP/每轨 LC/音频 LC 按需创建。
    assert!(
        !p.nodes()
            .any(|node| matches!(node.source, ChannelSource::Track(_)))
    );
    assert!(!p.nodes().any(|node| node.id == ChannelId::Master));
    assert_eq!(p.racks[9].id, RackId(9));
    assert_eq!(p.master.id, ChannelId::Master);
}

#[test]
fn audio_lc_ids_are_independent_from_source_channels() {
    let mut p = MixerParams::default();
    p.ensure_len();
    let audio0 = p.ensure_lc_node(ChannelSource::Audio(0));
    assert_ne!(audio0, LogicalChannelId(0));
    assert_eq!(p.audio_node(0).expect("audio 0").id, ChannelId::Lc(audio0));
}

#[test]
fn ensure_len_backfills_tables_and_fixes_ids() {
    let mut p = MixerParams::default();
    p.racks.clear();
    p.nodes = vec![MixerNode::logical_channel(
        LogicalChannelId(999),
        ChannelSource::Track(0),
    )];
    p.ensure_len();

    assert_eq!(p.racks.len(), CHANNEL_COUNT);
    assert!(
        p.channel_node(ChannelId::Lc(LogicalChannelId(999)))
            .is_some()
    );
    assert_eq!(p.master.id, ChannelId::Master);
}

/// 指向不存在通道的主输出会被 ensure_len 清理。
#[test]
fn ensure_len_prunes_dangling_routing() {
    let mut p = MixerParams::default();
    p.ensure_len();
    p.ensure_ap(0);
    let ap = p.ap_channel_id(0).expect("ap");
    let ghost = ChannelId::Lc(LogicalChannelId(TRACK_LC_BASE + 999));
    p.set_node_output(ap, Some(ghost));
    p.ensure_len();
    assert_eq!(p.node_output(ap), None);
}

#[test]
fn lc_display_numbers_are_stable_and_unique() {
    let mut p = MixerParams::default();
    p.ensure_len();
    let a = p.add_track_lc(0);
    let b = p.add_track_lc(1);
    let audio = p.ensure_lc_node(ChannelSource::Audio(0));
    let map = p.lc_display_map();
    assert_eq!(map.len(), 3);
    assert!(map.contains_key(&a));
    assert!(map.contains_key(&b));
    assert!(map.contains_key(&audio));
}

#[test]
fn unified_node_view_sees_all_tables_by_channel_id() {
    let mut p = MixerParams::default();
    p.ensure_len();
    let lc = p.add_track_lc(0);
    let audio = p.ensure_lc_node(ChannelSource::Audio(0));

    assert!(p.channel_node(ChannelId::Lc(lc)).is_some());
    assert!(p.channel_node(ChannelId::Lc(audio)).is_some());
    assert!(p.channel_node(ChannelId::Master).is_some());

    let ids: HashSet<ChannelId> = p.nodes().map(|node| node.channel_id()).collect();
    assert_eq!(
        ids.len(),
        p.nodes().count(),
        "统一节点视图不能出现重复 ChannelId"
    );
}

#[test]
fn mixed_lc_loses_freeze() {
    let mut p = MixerParams::default();
    p.ensure_len();
    let lc = p.add_track_lc(0);
    let source = p.add_track_lc(1);
    p.set_channel_send(ChannelId::Lc(source), ChannelId::Lc(lc), 0.5, false);
    p.track_node_mut(lc).expect("lc").freeze = Some(crate::node::FrozenAudio {
        source: "frozen".into(),
        source_revision: 1,
        sample_rate: 48_000,
        frames: 100,
    });
    p.ensure_len();
    assert!(
        p.track_node(lc).expect("lc").freeze.is_none(),
        "有 send 汇入的混合 LC 不能冻结"
    );
}

#[test]
fn channel_sends_are_any_to_any_and_zero_is_off() {
    let mut p = MixerParams::default();
    p.ensure_len();
    let source = p.add_track_lc(0);
    let target = p.add_track_lc(1);
    let source = ChannelId::Lc(source);
    let target = ChannelId::Lc(target);
    assert!(p.set_channel_send(source, target, 0.5, false));
    assert_eq!(p.channel_send_amount(source, target), 0.5);
    // 0 = OFF：必须删除条目，而不是保留极小值。
    assert!(p.set_channel_send(source, target, 0.0, false));
    assert_eq!(p.channel_send_amount(source, target), 0.0);
    assert!(p.channel_sends.is_empty());
    // Master 不能作为发送源；Master 可以作为目标。
    assert!(!p.set_channel_send(ChannelId::Master, target, 1.0, false));
    assert!(p.set_channel_send(source, ChannelId::Master, 1.0, false));
    assert_eq!(p.channel_send_amount(source, ChannelId::Master), 1.0);
}

#[test]
fn ap_groups_follow_midi_channel_and_are_unique() {
    let mut p = MixerParams::default();
    p.ensure_len();
    assert!(p.ensure_ap(3));
    let ap = p.ap_for_midi(3).expect("ap");
    assert_eq!(ap.id, ApGroupId(3));
    assert_eq!(ap.midi, (0, 3));
    assert_eq!(ap.rack, RackId(3));
    let ap_id = ChannelId::Ap(yinhe_types::ApChannelId::new(ApGroupId(3), 0));
    assert_eq!(p.ap_channel_id(3), Some(ap_id));
    assert_eq!(p.ap_node(3).expect("ap node").id, ap_id);
    assert!(!p.ensure_ap(3));
    assert_eq!(p.aps.len(), 1);
    assert!(p.remove_ap_for_midi(3));
    assert!(p.ap_for_midi(3).is_none());
    assert!(p.node(ap_id).is_none(), "删除 AP 时节点一起删除");
}

/// 统一节点入口：AP / 音频 / 每轨 LC / Master 各走各的存储，但 API 一致。
#[test]
fn node_strip_routes_each_namespace() {
    let mut p = MixerParams::default();
    p.ensure_len();
    p.ensure_ap(3);
    let lc = p.add_track_lc(0);
    let audio = p.ensure_lc_node(ChannelSource::Audio(7));

    for (node, gain) in [
        (p.ap_channel_id(3).expect("ap"), 0.3),
        (ChannelId::Lc(audio), 0.7),
        (ChannelId::Lc(lc), 0.5),
        (ChannelId::Master, 0.9),
    ] {
        let params = StripParams {
            gain,
            ..StripParams::default()
        };
        assert!(p.set_node_strip(node, params));
        assert_eq!(p.node_strip(node).gain, gain);
    }

    assert_eq!(p.audio_node(7).expect("audio").gain, 0.7);
    assert_eq!(p.node_strip(ChannelId::Lc(lc)).gain, 0.5);
    assert_eq!(p.master.gain, 0.9);
}

#[test]
fn node_inserts_and_output_route_each_namespace() {
    let mut p = MixerParams::default();
    p.ensure_len();
    p.ensure_ap(1);
    let lc = p.add_track_lc(0);
    let audio = p.ensure_lc_node(ChannelSource::Audio(2));
    let ap = p.ap_channel_id(1).expect("ap");

    assert!(p.node_inserts_mut(ap).is_some());
    assert!(p.node_inserts_mut(ChannelId::Lc(audio)).is_some());
    assert!(p.node_inserts_mut(ChannelId::Lc(lc)).is_some());
    assert!(p.node_inserts_mut(ChannelId::Master).is_some());

    assert!(p.set_node_output(ap, Some(ChannelId::Lc(lc))));
    assert_eq!(p.node_output(ap), Some(ChannelId::Lc(lc)));
    assert!(!p.set_node_output(ChannelId::Lc(lc), Some(ChannelId::Lc(lc))));
}

#[test]
fn rack_instruments_are_shared_by_id() {
    let mut p = MixerParams::default();
    let rack = RackId(7);
    let instruments = p.rack_instruments_mut(rack).expect("rack");
    instruments.push(InsertRef {
        plugin_id: "test".into(),
        ..InsertRef::default()
    });
    assert_eq!(p.rack_instruments(rack).len(), 1);
    assert_eq!(p.rack_instruments(rack)[0].plugin_id, "test");
}

#[test]
fn track_lcs_allocate_from_high_namespace_and_are_independent() {
    let mut p = MixerParams::default();
    let a = p.add_track_lc(0);
    let b = p.add_track_lc(1);
    assert_ne!(a, b);
    assert!(a.0 >= TRACK_LC_BASE);
    assert!(b.0 >= TRACK_LC_BASE);
    assert!(p.track_node(a).is_some());

    p.track_node_mut(a).expect("track lc").strip.gain = 0.25;
    assert_eq!(p.track_node(a).expect("track lc").gain, 0.25);
    assert_eq!(p.track_node(b).expect("track lc").gain, 1.0);

    p.remove_track_lc(a);
    assert!(p.track_node(a).is_none());
    assert!(p.track_node(b).is_some());
}
