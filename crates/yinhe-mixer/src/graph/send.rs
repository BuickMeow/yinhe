use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelSendTarget {
    Channel(usize),
    Master,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedChannelSend {
    pub target: ChannelSendTarget,
    pub amount: f32,
    pub pre_fader: bool,
}

pub(crate) fn route_channel_send_delayed(
    channels: &mut [ChannelChain],
    src: usize,
    send_idx: usize,
    master_l: &mut [f32],
    master_r: &mut [f32],
    any_solo: bool,
    frames: usize,
) {
    let Some(send) = channels[src].channel_sends.get(send_idx).copied() else {
        return;
    };
    let src_params = channels[src].strip.params;
    match send.target {
        ChannelSendTarget::Channel(dst) => {
            if dst <= src || dst >= channels.len() {
                return;
            }
            let dst_solo = channels[dst].strip.params.solo;
            if !channel_send_allowed(any_solo, &src_params, dst_solo) {
                return;
            }
            let (left, right) = channels.split_at_mut(dst);
            let src_chain = &mut left[src];
            let dst_chain = &mut right[0];
            src_chain.channel_send_delays[send_idx].add_delayed_into(
                &src_chain.buffers.left,
                &src_chain.buffers.right,
                &mut dst_chain.buffers.left,
                &mut dst_chain.buffers.right,
                send.amount,
                frames,
            );
        }
        ChannelSendTarget::Master => {
            if !channel_send_allowed(any_solo, &src_params, false) {
                return;
            }
            let src_chain = &mut channels[src];
            src_chain.channel_send_delays[send_idx].add_delayed_into(
                &src_chain.buffers.left,
                &src_chain.buffers.right,
                master_l,
                master_r,
                send.amount,
                frames,
            );
        }
    }
}

pub(crate) fn channel_send_allowed(any_solo: bool, src: &StripParams, dst_solo: bool) -> bool {
    !src.mute && (!any_solo || src.solo || dst_solo)
}
