use chia::protocol::Bytes32;
use chia_wallet_sdk::driver::SpendContext;

use crate::{
    CliError, Db, RewardDistributorCommitmentSlotValue, RewardDistributorEntrySlotValue,
    RewardDistributorRewardSlotValue, RewardDistributorSlotNonce, Slot,
};

pub async fn find_reward_slot_for_epoch(
    ctx: &mut SpendContext,
    db: &Db,
    launcher_id: Bytes32,
    epoch_start: u64,
    epoch_seconds: u64,
) -> Result<Option<Slot<RewardDistributorRewardSlotValue>>, CliError> {
    let mut next_slot_epoch = epoch_start;
    let mut reward_slot = None;

    let mut n = 0;
    while reward_slot.is_none() && n <= 52 {
        let nonce = RewardDistributorSlotNonce::REWARD.to_u64();
        let reward_slot_value_hashes = db
            .get_dig_indexed_slot_values_by_epoch_start(next_slot_epoch, nonce)
            .await?;

        // 0 or 1 value hashes per epoch
        for reward_slot_value_hash in reward_slot_value_hashes {
            if let Some(found_reward_slot) = db
                .get_slot::<RewardDistributorRewardSlotValue>(
                    ctx,
                    launcher_id,
                    nonce,
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
    clawback_ph: Bytes32,
    epoch_start: Option<u64>,
    reward_amount: Option<u64>,
) -> Result<Option<Slot<RewardDistributorCommitmentSlotValue>>, CliError> {
    let nonce = RewardDistributorSlotNonce::COMMITMENT.to_u64();
    let value_hashes = db
        .get_dig_indexed_slot_values_by_puzzle_hash(clawback_ph, nonce)
        .await?;

    let mut slot = None;
    for value_hash in value_hashes {
        let Some(commitment_slot) = db
            .get_slot::<RewardDistributorCommitmentSlotValue>(
                ctx,
                launcher_id,
                nonce,
                value_hash,
                0,
            )
            .await?
        else {
            continue;
        };

        if let Some(reward_amount) = reward_amount {
            if commitment_slot.info.value.rewards != reward_amount {
                continue;
            }
        }

        if let Some(epoch_start) = epoch_start {
            if commitment_slot.info.value.epoch_start != epoch_start {
                continue;
            }
        }

        slot = Some(commitment_slot);
        break;
    }

    Ok(slot)
}

pub async fn find_entry_slot_for_puzzle_hash(
    ctx: &mut SpendContext,
    db: &Db,
    launcher_id: Bytes32,
    entry_payout_puzzle_hash: Bytes32,
    entry_shares: Option<u64>,
) -> Result<Option<Slot<RewardDistributorEntrySlotValue>>, CliError> {
    let nonce = RewardDistributorSlotNonce::ENTRY.to_u64();
    let value_hashes = db
        .get_dig_indexed_slot_values_by_puzzle_hash(entry_payout_puzzle_hash, nonce)
        .await?;

    let mut slot = None;
    for value_hash in value_hashes {
        let Some(entry_slot) = db
            .get_slot::<RewardDistributorEntrySlotValue>(ctx, launcher_id, nonce, value_hash, 0)
            .await?
        else {
            continue;
        };

        if let Some(entry_shares) = entry_shares {
            if entry_slot.info.value.shares != entry_shares {
                continue;
            }
        }

        slot = Some(entry_slot);
        break;
    }

    Ok(slot)
}
