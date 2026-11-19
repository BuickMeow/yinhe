use super::*;

pub(crate) fn process_insert_chain(chain: &mut ChannelChain, frames: usize) {
    for insert in &mut chain.inserts {
        let max = insert.max_block_frames().max(1);
        if max >= frames {
            insert.process(&mut chain.buffers.left, &mut chain.buffers.right);
            continue;
        }
        let mut start = 0;
        while start < frames {
            let end = (start + max).min(frames);
            insert.process(
                &mut chain.buffers.left[start..end],
                &mut chain.buffers.right[start..end],
            );
            start = end;
        }
    }
    // PDC 已拆成 input_delay（块首）与 channel_send_delays / output_delay。
}

pub(crate) fn apply_channel_output(
    channels: &mut [ChannelChain],
    ci: usize,
    output: Option<usize>,
    to_master: bool,
    frames: usize,
) {
    let p = channels[ci].strip.params;
    match output {
        Some(dst) if dst > ci && dst < channels.len() => {
            let dst_solo = channels[dst].strip.params.solo;
            // 原条件：!mute && (!any_solo || solo || dst_solo)。
            // to_master 已包含 !mute && (!any_solo || solo)。
            if to_master || (!p.mute && dst_solo) {
                let (left, right) = channels.split_at_mut(dst);
                let src = &mut left[ci];
                src.output_delay.add_delayed_into(
                    &src.buffers.left,
                    &src.buffers.right,
                    &mut right[0].buffers.left,
                    &mut right[0].buffers.right,
                    1.0,
                    frames,
                );
            }
        }
        // 逆序/自环通道输出已经在块首用上一块快照汇入。
        Some(_) => {}
        // None = 主输出是 Master；音频只走 channel_sends 里的 Master send，
        // 不再直连累加（否则和 0 dB send 叠加成双份）。
        None => {}
    }
}
