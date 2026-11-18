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
    // PDC 已拆成 input_delay（块首）与 master_delay（Master 累加前）。
}

pub(crate) fn apply_channel_output(
    channels: &mut [ChannelChain],
    ci: usize,
    output: Option<usize>,
    to_master: bool,
    master_l: &mut [f32],
    master_r: &mut [f32],
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
        None => {
            if to_master {
                let chain = &mut channels[ci];
                // Master 输入 PDC：补到最长通道路径。
                chain
                    .master_delay
                    .process(&mut chain.buffers.left, &mut chain.buffers.right);
                let src = &chain.buffers;
                for f in 0..frames {
                    master_l[f] += src.left[f];
                    master_r[f] += src.right[f];
                }
            }
        }
    }
}
