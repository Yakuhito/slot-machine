use chia::{
    clvm_utils::ToTreeHash,
    protocol::Bytes32,
    puzzles::{cat::CatArgs, singleton::SingletonStruct},
};
use chia_wallet_sdk::{decode_address, encode_address, ChiaRpcClient, SpendContext};
use clvmr::serde::node_from_bytes;
use sage_api::{Amount, GetDerivations, SendCat};

use crate::{
    get_coinset_client, get_prefix, hex_string_to_bytes, hex_string_to_bytes32, parse_amount,
    sync_catalog, wait_for_coin, yes_no_prompt, CatNftMetadata, CatalogPrecommitValue,
    CatalogRegistryConstants, CliError, Db, DefaultCatMakerArgs, PrecommitLayer, SageClient,
};

#[allow(clippy::too_many_arguments)]
pub async fn catalog_register(
    tail_reveal_str: String,
    ticker: String,
    name: String,
    description: String,
    precision: u8,
    image_uris_str: String,
    image_hash_str: String,
    metadata_uris_str: String,
    metadata_hash_str: Option<String>,
    license_uris_str: String,
    license_hash_str: Option<String>,
    recipient_address: Option<String>,
    testnet11: bool,
    payment_asset_id_str: String,
    payment_cat_amount_str: Option<String>,
    fee_str: String,
) -> Result<(), CliError> {
    println!("Welcome to the CATalog registration process, issuer!");

    let mut ctx = SpendContext::new();
    let cli = get_coinset_client(testnet11);
    let catalog_constants = CatalogRegistryConstants::get(testnet11);
    let sage = SageClient::new()?;
    let db = Db::new().await?;

    let fee = parse_amount(fee_str.clone(), false)?;

    let initial_metadata = CatNftMetadata {
        ticker,
        name,
        description,
        precision,
        image_uris: image_uris_str.split(',').map(|s| s.to_string()).collect(),
        image_hash: hex_string_to_bytes32(&image_hash_str)?,
        metadata_uris: metadata_uris_str
            .split(',')
            .map(|s| s.to_string())
            .collect(),
        metadata_hash: if let Some(metadata_hash_str) = metadata_hash_str {
            Some(hex_string_to_bytes32(&metadata_hash_str)?)
        } else {
            None
        },
        license_uris: license_uris_str.split(',').map(|s| s.to_string()).collect(),
        license_hash: if let Some(license_hash_str) = license_hash_str {
            Some(hex_string_to_bytes32(&license_hash_str)?)
        } else {
            None
        },
    };

    let payment_asset_id = hex_string_to_bytes32(&payment_asset_id_str)?;

    print!("First, let's sync CATalog... ");
    let catalog = sync_catalog(&cli, &db, &mut ctx, catalog_constants).await?;
    println!("done.");

    let recipient_address = if let Some(provided_recipient_address) = recipient_address {
        provided_recipient_address
    } else {
        let derivation_resp = sage
            .get_derivations(GetDerivations {
                hardened: false,
                offset: 0,
                limit: 1,
            })
            .await?;
        derivation_resp.derivations[0].address.clone()
    };

    let tail_ptr = node_from_bytes(&mut ctx.allocator, &hex_string_to_bytes(&tail_reveal_str)?)?;
    let registered_asset_id: Bytes32 = ctx.tree_hash(tail_ptr).into();

    if DefaultCatMakerArgs::curry_tree_hash(payment_asset_id.tree_hash().into())
        != catalog.info.state.cat_maker_puzzle_hash.into()
    {
        yes_no_prompt("CAT maker puzzle hash doesn't correspond to the given payment asset ID. Registration will NOT work unless the price singleton changes the registry's state. Continue at your own risk?")?;
    }

    let mut payment_cat_amount = catalog.info.state.registration_price;
    if let Some(payment_cat_amount_str) = payment_cat_amount_str {
        let parsed_payment_cat_amount = parse_amount(payment_cat_amount_str, true)?;
        if parsed_payment_cat_amount != payment_cat_amount {
            yes_no_prompt("Payment CAT amount is different from the specified registration price. Registration will likely fail. Continue at your own risk?")?;
            payment_cat_amount = parsed_payment_cat_amount;
        }
    }

    let recipient_puzzle_hash = Bytes32::new(decode_address(&recipient_address)?.0);

    let initial_nft_puzzle_ptr = CatalogPrecommitValue::<()>::initial_inner_puzzle(
        &mut ctx,
        recipient_puzzle_hash,
        initial_metadata.clone(),
    )?;

    let precommit_value = CatalogPrecommitValue::with_default_cat_maker(
        payment_asset_id.tree_hash(),
        ctx.tree_hash(initial_nft_puzzle_ptr).into(),
        tail_ptr,
    );
    let precommit_value_ptr = ctx.alloc(&precommit_value)?;

    let precommit_inner_puzzle_hash = PrecommitLayer::<()>::puzzle_hash(
        SingletonStruct::new(catalog_constants.launcher_id)
            .tree_hash()
            .into(),
        catalog_constants.relative_block_height,
        catalog_constants.precommit_payout_puzzle_hash,
        recipient_puzzle_hash,
        ctx.tree_hash(precommit_value_ptr),
    );

    let precomit_puzzle_hash =
        CatArgs::curry_tree_hash(payment_asset_id, precommit_inner_puzzle_hash);

    let Some(potential_precommit_coin_records) = cli
        .get_coin_records_by_hint(precommit_inner_puzzle_hash.into(), None, None, Some(false))
        .await?
        .coin_records
    else {
        return Err(CliError::Custom(
            "Could not check whether precommit coin exists".to_string(),
        ));
    };

    let precommit_coin_record = potential_precommit_coin_records.iter().find(|cr| {
        cr.coin.puzzle_hash == precomit_puzzle_hash.into() && cr.coin.amount == payment_cat_amount
    });

    if let Some(_precommit_coin_record) = precommit_coin_record {
        println!("Precommitment coin found!");
        todo!("implement this path")
    }

    println!("Registered asset ID: {}", hex::encode(registered_asset_id));

    println!("Have one last look at the initial metadata:");
    initial_metadata.pretty_print("  ");

    println!("The NFT will be minted to the following address:");
    println!("  {}", recipient_address);

    println!("\nCONFIRM THE ADDRESS IS CORRECT - NFT CANNOT BE RECOVERED AFTER REGISTRATION\n");

    println!(
        "Your wallet will send {} mojos of the payment asset with a fee of {} XCH ({} mojos)",
        payment_cat_amount, fee_str, fee
    );

    yes_no_prompt("Continue with registration?")?;

    let precommit_coin_address =
        encode_address(precommit_inner_puzzle_hash.into(), &get_prefix(testnet11))?;
    let send_resp = sage
        .send_cat(SendCat {
            asset_id: format!("0x{}", hex::encode(payment_asset_id)),
            address: precommit_coin_address,
            amount: Amount::Number(payment_cat_amount),
            fee: Amount::Number(fee),
            memos: vec![],
            auto_submit: true,
        })
        .await?;
    println!("Transaction sent.");

    wait_for_coin(
        &cli,
        hex_string_to_bytes32(&send_resp.summary.inputs[0].coin_id)?,
        true,
    )
    .await?;
    println!("Confirmed!");

    Ok(())
}
