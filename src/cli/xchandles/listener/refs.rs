use chia_protocol::Bytes32;
use chia_wallet_sdk::driver::XchandlesActionLog;
use chia_wallet_sdk::types::puzzles::XchandlesHandleSlotValue;

/// A singleton launcher newly referenced by a registry transition, with the expected
/// full/inner puzzle relationship committed by that action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SingletonReference {
    pub launcher_id: Bytes32,
    pub expected_full_puzzle_hash: Bytes32,
    pub expected_inner_puzzle_hash: Bytes32,
}

/// Extract Owner/Resolved Singleton references introduced by an action log entry.
pub fn references_from_action_log(log: &XchandlesActionLog) -> Vec<SingletonReference> {
    match log {
        XchandlesActionLog::Register(reg) => refs_from_transition(
            &[reg.spent_left_slot, reg.spent_right_slot],
            &[
                reg.created_left_slot,
                reg.created_handle_slot,
                reg.created_right_slot,
            ],
            reg.created_handle_slot.owner_launcher_id,
            reg.created_handle_slot.resolved_launcher_id,
            reg.owner_full_puzzle_hash,
            reg.resolved_full_puzzle_hash,
            reg.owner_inner_puzzle_hash,
            reg.resolved_inner_puzzle_hash,
        ),
        XchandlesActionLog::Expire(exp) => refs_from_transition(
            &[exp.spent_slot],
            &[exp.created_slot],
            exp.created_slot.owner_launcher_id,
            exp.created_slot.resolved_launcher_id,
            exp.owner_full_puzzle_hash,
            exp.resolved_full_puzzle_hash,
            exp.owner_inner_puzzle_hash,
            exp.resolved_inner_puzzle_hash,
        ),
        XchandlesActionLog::ExecuteUpdate(upd) => refs_from_transition(
            &[upd.spent_handle_slot],
            &[upd.created_slot],
            upd.created_slot.owner_launcher_id,
            upd.created_slot.resolved_launcher_id,
            upd.owner_full_puzzle_hash,
            upd.resolved_full_puzzle_hash,
            upd.owner_inner_puzzle_hash,
            upd.resolved_inner_puzzle_hash,
        ),
        _ => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn refs_from_transition(
    spent: &[XchandlesHandleSlotValue],
    created: &[XchandlesHandleSlotValue],
    owner_launcher: Bytes32,
    resolved_launcher: Bytes32,
    owner_full: Bytes32,
    resolved_full: Option<Bytes32>,
    owner_inner: Bytes32,
    resolved_inner: Bytes32,
) -> Vec<SingletonReference> {
    let previously: Vec<Bytes32> = spent
        .iter()
        .flat_map(|s| [s.owner_launcher_id, s.resolved_launcher_id])
        .collect();

    let mut out = Vec::new();
    let newly_owner = created
        .iter()
        .any(|c| c.owner_launcher_id == owner_launcher)
        && !previously.contains(&owner_launcher);
    if newly_owner {
        out.push(SingletonReference {
            launcher_id: owner_launcher,
            expected_full_puzzle_hash: owner_full,
            expected_inner_puzzle_hash: owner_inner,
        });
    }

    if owner_launcher != resolved_launcher {
        let newly_resolved = created
            .iter()
            .any(|c| c.resolved_launcher_id == resolved_launcher)
            && !previously.contains(&resolved_launcher);
        if newly_resolved {
            if let Some(full) = resolved_full {
                out.push(SingletonReference {
                    launcher_id: resolved_launcher,
                    expected_full_puzzle_hash: full,
                    expected_inner_puzzle_hash: resolved_inner,
                });
            }
        }
    } else if newly_owner {
        // Owner == resolved: one reference already recorded.
    }

    out
}

/// Launchers that disappear from a spent→created Handle slot transition.
pub fn dereferenced_launchers(
    spent: &XchandlesHandleSlotValue,
    created: &XchandlesHandleSlotValue,
) -> Vec<Bytes32> {
    let mut out = Vec::new();
    let still = |id: Bytes32| id == created.owner_launcher_id || id == created.resolved_launcher_id;
    if !still(spent.owner_launcher_id) {
        out.push(spent.owner_launcher_id);
    }
    if spent.resolved_launcher_id != spent.owner_launcher_id && !still(spent.resolved_launcher_id) {
        out.push(spent.resolved_launcher_id);
    }
    out
}
