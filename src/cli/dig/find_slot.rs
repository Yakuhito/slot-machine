use chia::protocol::Bytes32;
use chia_wallet_sdk::SpendContext;

use crate::{CliError, Db, DigRewardSlotValue, DigSlotNonce, Slot};

pub async fn find_reward_slot_for_epoch(
    ctx: &mut SpendContext,
    db: &Db,
    launcher_id: Bytes32,
    epoch_start: u64,
    epoch_seconds: u64,
) -> Result<Option<Slot<DigRewardSlotValue>>, CliError> {
    let mut next_slot_epoch = epoch_start;
    let mut reward_slot = None;

    let mut n = 0;
    while reward_slot.is_none() && n <= 52 {
        let reward_slot_value_hashes = db
            .get_dig_indexed_slot_values_by_epoch_start(next_slot_epoch)
            .await?;
        for reward_slot_value_hash in reward_slot_value_hashes {
            if let Some(found_reward_slot) = db
                .get_slot::<DigRewardSlotValue>(
                    &mut ctx.allocator,
                    launcher_id,
                    DigSlotNonce::REWARD.to_u64(),
                    reward_slot_value_hash,
                    0,
                )
                .await?
            {
                reward_slot = Some(found_reward_slot);
                break;
            }
        }

        next_slot_epoch -= epoch_seconds;
        n += 1;
    }

    Ok(reward_slot)
}

pub async fn find_commitment_slot_for_puzzle_hash(
    ctx: &mut SpendContext,
    db: &Db,
    launcher_id: Bytes32,
    puzzle_hash: Bytes32,
    epoch_start: Option<u64>,
    reward_amount: Option<u64>,
) -> Result<Option<Slot<DigCommitmentSlotValue>>, CliError> {
    todo!()
}
