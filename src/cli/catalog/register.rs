use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
use chia_wallet_sdk::SpendContext;
use clvmr::serde::node_from_bytes;
use sage_api::GetDerivations;

use crate::{
    get_coinset_client, hex_string_to_bytes, hex_string_to_bytes32, parse_amount, sync_catalog,
    yes_no_prompt, CatNftMetadata, CatalogRegistryConstants, CliError, Db, DefaultCatMakerArgs,
    SageClient,
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

    println!("Registered asset ID: {}", hex::encode(registered_asset_id));

    println!("Have one last look at the initial metadata:");
    initial_metadata.pretty_print("  ");

    println!("The NFT will be minted to the following address:");
    println!("  {}", recipient_address);

    println!("\nCONFIRM THE ADDRESS IS CORRECT - NFT CANNOT BE RECOVERED AFTER REGISTRATION\n");

    println!("A one-sided offer will be created to mint the precommitment coin. It will contain:");
    println!("  {} mojos of the payment asset", payment_cat_amount);
    println!("  {} XCH ({} mojos) fee", fee_str, fee);

    yes_no_prompt("Continue with registration?")?;

    Ok(())
}
