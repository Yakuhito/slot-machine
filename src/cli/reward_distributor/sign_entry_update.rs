use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes, Bytes32},
};
use clvmr::Allocator;

use crate::{
    get_constants, hex_string_to_bytes32, multisig_sign_thing_finish, multisig_sign_thing_start,
    CliError, Db, MedievalVault,
};

pub async fn reward_distributor_sign_entry_update(
    launcher_id_str: String,
    entry_payout_puzzle_hash_str: String,
    entry_shares: u64,
    my_pubkey_str: String,
    remove_entry: bool,
    testnet11: bool,
    debug: bool,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let entry_payout_puzzle_hash = hex_string_to_bytes32(&entry_payout_puzzle_hash_str)?;

    println!("\nGetting distributor constants... ");
    let db = Db::new(true).await?;

    let mut temp_allocator = Allocator::new();
    let distributor_constants = db
        .get_reward_distributor_configuration(&mut temp_allocator, launcher_id)
        .await?
        .ok_or(CliError::Custom(
            "Could not get reward distributor constants - try running another command to sync it first".to_string(),
        ))?;

    let (my_pubkey, mut ctx, _client, medieval_vault) = multisig_sign_thing_start(
        my_pubkey_str,
        hex::encode(distributor_constants.manager_launcher_id),
        testnet11,
    )
    .await?;

    if remove_entry {
        println!("\nYou'll *REMOVE* the following entry from the reward list:");
    } else {
        println!("\nYou'll *ADD* a new entry with the following parameters:");
    }
    println!(
        "  Entry payout puzzle hash: {}",
        hex::encode(entry_payout_puzzle_hash)
    );
    println!("  Entry shares: {}", entry_shares);

    let message: Bytes32 = (entry_payout_puzzle_hash, entry_shares).tree_hash().into();
    let mut message: Vec<u8> = message.to_vec();
    if remove_entry {
        message.insert(0, b'r');
    } else {
        message.insert(0, b'a');
    }

    let delegated_puzzle = MedievalVault::delegated_puzzle_for_flexible_send_message::<Bytes>(
        &mut ctx,
        Bytes::new(message),
        launcher_id,
        medieval_vault.coin,
        &medieval_vault.info,
        get_constants(testnet11).genesis_challenge,
    )
    .map_err(CliError::Driver)?;

    multisig_sign_thing_finish(
        &mut ctx,
        delegated_puzzle,
        &medieval_vault,
        my_pubkey,
        testnet11,
        debug,
    )
    .await
}
