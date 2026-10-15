//! The phase of a script in a job: the mapping between the profile type and the protocol value
//! (FR-HK-05).

use restichub_proto::agent::v1::HookPhase as ProtoPhase;

use crate::profile::HookPhase;

fn phase_from_proto(value: i32) -> Option<HookPhase> {
    match ProtoPhase::try_from(value).ok()? {
        ProtoPhase::Before => Some(HookPhase::Before),
        ProtoPhase::After => Some(HookPhase::After),
        ProtoPhase::OnSuccess => Some(HookPhase::OnSuccess),
        ProtoPhase::OnError => Some(HookPhase::OnError),
        ProtoPhase::Finally => Some(HookPhase::Finally),
        ProtoPhase::Unspecified => None,
    }
}

/// Maps a phase to its protocol value.
#[must_use]
pub fn phase_to_proto(phase: HookPhase) -> ProtoPhase {
    match phase {
        HookPhase::Before => ProtoPhase::Before,
        HookPhase::After => ProtoPhase::After,
        HookPhase::OnSuccess => ProtoPhase::OnSuccess,
        HookPhase::OnError => ProtoPhase::OnError,
        HookPhase::Finally => ProtoPhase::Finally,
    }
}

/// The phase of a protocol value, or `None` for an unspecified or unknown value.
#[must_use]
pub fn phase_of(value: i32) -> Option<HookPhase> {
    phase_from_proto(value)
}
