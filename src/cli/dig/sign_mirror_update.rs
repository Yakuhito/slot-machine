use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes, Bytes32},
};

use crate::{
    get_constants, hex_string_to_bytes32, multisig_sign_thing_finish, multisig_sign_thing_start,
    sync_distributor, CliError, Db, MedievalVault,
};

pub async fn dig_sign_mirror_update(
    launcher_id_str: String,
    mirror_payout_puzzle_hash_str: String,
    mirror_shares: u64,
    my_pubkey_str: String,
    remove_mirror: bool,
    testnet11: bool,
    debug: bool,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let mirror_payout_puzzle_hash = hex_string_to_bytes32(&mirror_payout_puzzle_hash_str)?;

    let (my_pubkey, mut ctx, client, medieval_vault) =
        multisig_sign_thing_start(my_pubkey_str, launcher_id_str, testnet11).await?;

    println!("\nSyncing reward distributor... ");
    let db = Db::new(true).await?;
    let reward_distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;
    println!("Done!");

    if remove_mirror {
        println!("You'll *ADD* a new mirror reward entry with the following parameters:");
    } else {
        println!("You'll *REMOVE* the following mirror from the rewarded mirror lists:");
    }
    println!(
        "  Mirror payout puzzle hash: {}",
        hex::encode(mirror_payout_puzzle_hash)
    );
    println!("  Mirror shares: {}", mirror_shares);

    let message: Bytes32 = (mirror_payout_puzzle_hash, mirror_shares)
        .tree_hash()
        .into();
    let mut message: Vec<u8> = message.to_vec();
    if remove_mirror {
        message.insert(0, b'r');
    } else {
        message.insert(0, b'a');
    }

    let delegated_puzzle = MedievalVault::delegated_puzzle_for_flexible_send_message::<Bytes>(
        &mut ctx,
        Bytes::new(message),
        reward_distributor.info.constants.validator_launcher_id,
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
