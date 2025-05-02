use chia::protocol::Bytes32;
use chia_puzzle_types::singleton::LauncherSolution;
use chia_puzzles::SINGLETON_LAUNCHER_HASH;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{DriverError, Layer, Puzzle, SingletonLayer, SpendContext},
};
use clvmr::NodePtr;

use crate::{CliError, Verification, VerificationInfo, VerificationLauncherKVList};

pub async fn sync_verifications(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    data_hash: Bytes32,
    issuer_launcher_ids_filter: Option<Vec<Bytes32>>,
    print: bool,
) -> Result<Vec<Verification>, CliError> {
    if print {
        println!("Looking for on-chain attestation(s)...");
    }

    let possible_coin_records = client
        .get_coin_records_by_hint(data_hash, None, None, Some(true))
        .await?
        .coin_records
        .ok_or(CliError::Driver(DriverError::MissingHint))?;

    let mut verifications = Vec::new();

    for coin_record in possible_coin_records {
        if coin_record.coin.puzzle_hash != SINGLETON_LAUNCHER_HASH.into()
            || coin_record.coin.amount != 0
            || !coin_record.spent
        {
            continue;
        }

        let Some(coin_spend) = client
            .get_puzzle_and_solution(
                coin_record.coin.coin_id(),
                Some(coin_record.spent_block_index),
            )
            .await?
            .coin_solution
        else {
            continue;
        };

        let solution_ptr = ctx.alloc(&coin_spend.solution)?;
        let solution = ctx.extract::<LauncherSolution<VerificationLauncherKVList>>(solution_ptr)?;
        let verification = Verification::after_mint(
            coin_record.coin.parent_coin_info,
            VerificationInfo {
                launcher_id: coin_record.coin.coin_id(),
                revocation_singleton_launcher_id: solution
                    .key_value_list
                    .revocation_singleton_launcher_id,
                verified_data: solution.key_value_list.verified_data,
            },
        );

        if verification.coin.puzzle_hash != solution.singleton_puzzle_hash {
            continue;
        }

        // Lastly, also check parent is singleton with launcher id = revocation launcher id
        let Some(parent_coin_spend) = client
            .get_puzzle_and_solution(
                coin_record.coin.parent_coin_info,
                Some(coin_record.confirmed_block_index),
            )
            .await?
            .coin_solution
        else {
            continue;
        };

        let parent_puzzle_ptr = ctx.alloc(&parent_coin_spend.puzzle_reveal)?;
        let parent_puzzle = Puzzle::parse(ctx, parent_puzzle_ptr);
        let Some(parent_puzzle) = SingletonLayer::<NodePtr>::parse_puzzle(ctx, parent_puzzle)?
        else {
            continue;
        };

        if parent_puzzle.launcher_id != verification.info.revocation_singleton_launcher_id {
            continue;
        }

        if let Some(filters) = &issuer_launcher_ids_filter {
            if !filters.contains(&verification.info.launcher_id) {
                continue;
            }
        }

        if print {
            println!(
                "\nVerification 0x{}",
                hex::encode(verification.info.launcher_id)
            );
            println!(
                "  Revocation singleton launcher id: 0x{}",
                hex::encode(verification.info.revocation_singleton_launcher_id)
            );
            println!("  Version: {}", verification.info.verified_data.version);
            println!("  Comment: {}", verification.info.verified_data.comment);
        }

        // Warning: Anyone can create an 'unspent revocation' with the correct puzzle hash and amount.
        // For this fast check to be secure, we need to ensure the parent has the same puzzle hash as well
        //   (i.e., it is a singleton with the right launcher id / revocation layer)
        let mut revoked = true;

        let coin_records = client
            .get_coin_records_by_hint(verification.info.launcher_id, None, None, Some(true))
            .await?
            .coin_records
            .unwrap_or_default();
        if coin_records.is_empty() {
            // Just launched
            revoked = false;
        }

        for coin_record in coin_records {
            if coin_record.coin.puzzle_hash != verification.coin.puzzle_hash
                || coin_record.coin.amount != 1
                || coin_record.spent
            {
                continue;
            }

            let Some(parent) = client
                .get_coin_record_by_name(coin_record.coin.parent_coin_info)
                .await?
                .coin_record
            else {
                continue;
            };

            if parent.coin.puzzle_hash != verification.coin.puzzle_hash
                && parent.coin.coin_id() != verification.info.launcher_id
            {
                continue;
            }

            revoked = false;
        }

        if print {
            println!("  Revoked: {}", revoked);
        }

        if !revoked {
            verifications.push(verification);
        }
    }

    if print {
        println!("\nDone.");
    }

    Ok(verifications)
}
