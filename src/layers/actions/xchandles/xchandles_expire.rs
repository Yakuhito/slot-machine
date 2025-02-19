use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
};
use chia_wallet_sdk::{announcement_id, Conditions, DriverError, Spend, SpendContext};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, DefaultCatMakerArgs, PrecommitCoin, PrecommitLayer, Slot, SpendContextExt,
    XchandlesConstants, XchandlesPrecommitValue, XchandlesRegistry, XchandlesSlotValue,
};

use super::{XchandlesFactorPricingPuzzleArgs, XchandlesFactorPricingSolution};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesExpireAction {
    pub launcher_id: Bytes32,
    pub relative_block_height: u32,
    pub payout_puzzle_hash: Bytes32,
}

impl ToTreeHash for XchandlesExpireAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesExpireActionArgs::curry_tree_hash(
            self.launcher_id,
            self.relative_block_height,
            self.payout_puzzle_hash,
        )
    }
}

impl Action<XchandlesRegistry> for XchandlesExpireAction {
    fn from_constants(launcher_id: Bytes32, constants: &XchandlesConstants) -> Self {
        Self {
            launcher_id,
            relative_block_height: constants.relative_block_height,
            payout_puzzle_hash: constants.precommit_payout_puzzle_hash,
        }
    }
}

impl XchandlesExpireAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_expire_puzzle()?,
            args: XchandlesExpireActionArgs::new(
                self.launcher_id,
                self.relative_block_height,
                self.payout_puzzle_hash,
            ),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    pub fn get_slot_value_from_solution(
        &self,
        ctx: &SpendContext,
        old_slot_value: XchandlesSlotValue,
        precommit_coin_value: XchandlesPrecommitValue,
        solution: NodePtr,
    ) -> Result<XchandlesSlotValue, DriverError> {
        let solution = XchandlesExpireActionSolution::<
            NodePtr,
            (),
            NodePtr,
            XchandlesExponentialPremiumRenewPuzzleSolution<XchandlesFactorPricingSolution>,
        >::from_clvm(&ctx.allocator, solution)?;

        Ok(XchandlesSlotValue {
            handle_hash: old_slot_value.handle_hash,
            neighbors: old_slot_value.neighbors,
            expiration: precommit_coin_value.start_time
                + 366
                    * 24
                    * 60
                    * 60
                    * solution
                        .expired_handle_pricing_puzzle_solution
                        .pricing_program_solution
                        .num_years,
            owner_launcher_id: precommit_coin_value.owner_launcher_id,
            resolved_launcher_id: precommit_coin_value.resolved_launcher_id,
        })
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        registry: &mut XchandlesRegistry,
        slot: Slot<XchandlesSlotValue>,
        num_years: u64,
        base_handle_price: u64,
        precommit_coin: PrecommitCoin<XchandlesPrecommitValue>,
    ) -> Result<(Conditions, Slot<XchandlesSlotValue>), DriverError> {
        // spend slot
        let Some(slot_value) = slot.info.value else {
            return Err(DriverError::Custom("Missing slot value".to_string()));
        };

        let my_inner_puzzle_hash: Bytes32 = registry.info.inner_puzzle_hash().into();
        slot.spend(ctx, my_inner_puzzle_hash)?;

        // announcement is simply premcommitment coin inner ph
        let expire_ann: Bytes32 = precommit_coin.inner_puzzle_hash;

        // spend precommit coin
        precommit_coin.spend(
            ctx,
            1, // mode 1 = register/expire (use value)
            my_inner_puzzle_hash,
        )?;

        // finally, spend self
        let action_solution = XchandlesExpireActionSolution {
            cat_maker_puzzle_reveal: DefaultCatMakerArgs::get_puzzle(
                ctx,
                precommit_coin.asset_id.tree_hash().into(),
            )?,
            cat_maker_puzzle_solution: (),
            expired_handle_pricing_puzzle_reveal:
                XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(
                    ctx,
                    base_handle_price,
                    1000,
                )?
                .get_puzzle(ctx)?,
            expired_handle_pricing_puzzle_solution:
                XchandlesExponentialPremiumRenewPuzzleSolution::<XchandlesFactorPricingSolution> {
                    buy_time: precommit_coin.value.start_time,
                    pricing_program_solution: XchandlesFactorPricingSolution {
                        current_expiration: slot_value.expiration,
                        handle: precommit_coin.value.secret_and_handle.handle.clone(),
                        num_years,
                    },
                },
            refund_puzzle_hash_hash: precommit_coin.refund_puzzle_hash.tree_hash().into(),
            secret_hash: precommit_coin
                .value
                .secret_and_handle
                .secret
                .tree_hash()
                .into(),
            neighbors_hash: slot_value.neighbors.tree_hash().into(),
            old_rest_hash: slot_value.launcher_ids_data_hash().into(),
            new_rest_hash: clvm_tuple!(
                precommit_coin.value.owner_launcher_id,
                precommit_coin.value.resolved_launcher_id
            )
            .tree_hash()
            .into(),
        }
        .to_clvm(&mut ctx.allocator)?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        registry.insert(Spend::new(action_puzzle, action_solution));
        let new_slot_value = self.get_slot_value_from_solution(
            ctx,
            slot_value,
            precommit_coin.value,
            action_solution,
        )?;

        let mut expire_ann: Vec<u8> = expire_ann.to_vec();
        expire_ann.insert(0, b'x');
        Ok((
            Conditions::new()
                .assert_puzzle_announcement(announcement_id(registry.coin.puzzle_hash, expire_ann)),
            registry.created_slot_values_to_slots(vec![new_slot_value])[0],
        ))
    }
}

pub const XCHANDLES_EXPIRE_PUZZLE: [u8; 1184] =
    hex!("ff02ffff01ff02ffff03ffff22ffff09ffff02ff36ffff04ff02ffff04ff4fff80808080ff2780ffff09ffff02ff36ffff04ff02ffff04ff82016fff80808080ff778080ffff01ff04ff17ffff02ff2effff04ff02ffff04ff4fffff04ff81afffff04ffff05ffff02ff82016fff8202ef8080ffff04ffff02ff2affff04ff02ffff04ff05ffff04ff8205efffff04ffff0bffff0102ffff0bffff0101ffff0bffff0102ffff0bffff0102ff27ffff02ff36ffff04ff02ffff04ff81afff8080808080ffff0bffff0102ff77ffff02ff36ffff04ff02ffff04ff8202efff80808080808080ffff0bffff0102ffff0bffff0102ff820befffff0bffff0101ff8216ef8080ffff0bffff0102ffff0bffff0101ff8204ef80ff823fef808080ff808080808080ffff04ffff04ffff04ff10ffff04ff8204efff808080ffff04ffff04ff10ffff04ff820aefff808080ffff04ffff02ff3effff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ffff0bffff0101ff8216ef80ffff04ff8217efffff04ff820aefffff04ff822fefff80808080808080ff8080808080ffff04ffff02ff12ffff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ffff0bffff0101ff8216ef80ffff04ff8217efffff04ffff10ffff06ffff02ff82016fff8202ef8080ff8204ef80ffff04ff823fefff80808080808080ff8080808080ff8080808080ff808080808080808080ffff01ff088080ff0180ffff04ffff01ffffff51ff333effff4202ffff02ffff03ff05ffff01ff0bff81fcffff02ff26ffff04ff02ffff04ff09ffff04ffff02ff2cffff04ff02ffff04ff0dff80808080ff808080808080ffff0181dc80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff04ff28ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff81bcffff02ff26ffff04ff02ffff04ff05ffff04ffff02ff2cffff04ff02ffff04ff07ff80808080ff808080808080ff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ff0bffff0bffff0102ffff0bffff0101ff1780ff2f808080ffffff0bff34ffff0bff34ff81dcff0580ffff0bff34ff0bff819c8080ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff36ffff04ff02ffff04ff09ff80808080ffff02ff36ffff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ffff04ffff04ff24ffff04ffff0113ffff04ffff0101ffff04ffff02ff05ffff04ff2fffff04ff0bff80808080ffff04ff17ff808080808080ffff04ffff04ff38ffff04ffff0effff0178ff2f80ff808080ff5f8080ff04ff24ffff04ffff0112ffff04ff80ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_EXPIRE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    848620a1cb62b50b3640fb8c197c0b63203cea9803819dcdc1d314ff87a7ed05
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesExpireActionArgs {
    pub precommit_1st_curry_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesExpireActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            precommit_1st_curry_hash: PrecommitLayer::<()>::first_curry_hash(
                SingletonStruct::new(launcher_id).tree_hash().into(),
                relative_block_height,
                payout_puzzle_hash,
            )
            .into(),
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id, None).into(),
        }
    }
}

impl XchandlesExpireActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_EXPIRE_PUZZLE_HASH,
            args: XchandlesExpireActionArgs::new(
                launcher_id,
                relative_block_height,
                payout_puzzle_hash,
            ),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesExpireActionSolution<CMP, CMS, P, S> {
    pub cat_maker_puzzle_reveal: CMP,
    pub cat_maker_puzzle_solution: CMS,
    pub expired_handle_pricing_puzzle_reveal: P,
    pub expired_handle_pricing_puzzle_solution: S,
    pub refund_puzzle_hash_hash: Bytes32,
    pub secret_hash: Bytes32,
    pub neighbors_hash: Bytes32,
    pub old_rest_hash: Bytes32,
    #[clvm(rest)]
    pub new_rest_hash: Bytes32,
}

pub const XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE: [u8; 353] =
    hex!("ff02ffff01ff04ffff10ffff05ffff02ff05ffff04ff82017fff8201ff808080ffff02ff0effff04ff02ffff04ffff02ff0affff04ff02ffff04ff2fffff04ff5fffff04ffff0101ffff04ffff05ffff14ffff12ffff0183010000ffff3dffff11ff81bfff82017f80ff048080ff048080ffff04ffff05ffff14ff0bffff17ffff0101ffff05ffff14ffff11ff81bfff82017f80ff048080808080ff8080808080808080ffff04ff17ff808080808080ffff06ffff02ff05ffff04ff82017fff8201ff80808080ffff04ffff01ff83015180ffff02ffff03ff0bffff01ff02ff0affff04ff02ffff04ff05ffff04ff1bffff04ffff17ff17ffff010180ffff04ff2fffff04ffff02ffff03ffff18ff2fff1780ffff01ff05ffff14ffff12ff5fff1380ff058080ffff015f80ff0180ff8080808080808080ffff015f80ff0180ff02ffff03ffff15ff05ff0b80ffff01ff11ff05ff0b80ff8080ff0180ff018080");

pub const XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    946e180f6006f656200ee6241d03de5bc5c3c6f7606a2159df598ccfd47d740a
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesExponentialPremiumRenewPuzzleArgs<P> {
    pub base_program: P,
    pub start_premium: u64,
    pub end_value: u64,
    pub precision: u64,
    pub bits_list: Vec<u64>,
}

pub const PREMIUM_PRECISION: u64 = 1_000_000_000_000_000_000; // 10^18

// https://github.com/ensdomains/ens-contracts/blob/master/contracts/ethregistrar/ExponentialPremiumPriceOracle.sol
pub const PREMIUM_BITS_LIST: [u64; 16] = [
    999989423469314432, // 0.5 ^ 1/65536 * (10 ** 18)
    999978847050491904, // 0.5 ^ 2/65536 * (10 ** 18)
    999957694548431104,
    999915390886613504,
    999830788931929088,
    999661606496243712,
    999323327502650752,
    998647112890970240,
    997296056085470080,
    994599423483633152,
    989228013193975424,
    978572062087700096,
    957603280698573696,
    917004043204671232,
    840896415253714560,
    707106781186547584,
];

impl XchandlesExponentialPremiumRenewPuzzleArgs<NodePtr> {
    pub fn get_start_premium(scale_factor: u64) -> u64 {
        100000000 * scale_factor // start auction at $100 million
    }

    pub fn get_end_value(scale_factor: u64) -> u64 {
        // 100000000 * 10 ** 18 // 2 ** 28 = 372529029846191406
        (372529029846191406_u128 * scale_factor as u128 / 1_000_000_000_000_000_000) as u64
    }

    // A scale factor is how many units of the payment token equate to $1
    // For exampe, you'd use scale_factor=1000 for wUSDC.b
    pub fn from_scale_factor(
        ctx: &mut SpendContext,
        base_price: u64,
        scale_factor: u64,
    ) -> Result<Self, DriverError> {
        Ok(Self {
            base_program: XchandlesFactorPricingPuzzleArgs::get_puzzle(ctx, base_price)?,
            start_premium: Self::get_start_premium(scale_factor),
            end_value: Self::get_end_value(scale_factor),
            precision: PREMIUM_PRECISION,
            bits_list: PREMIUM_BITS_LIST.to_vec(),
        })
    }

    pub fn curry_tree_hash(base_price: u64, scale_factor: u64) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE_HASH,
            args: XchandlesExponentialPremiumRenewPuzzleArgs::<TreeHash> {
                base_program: XchandlesFactorPricingPuzzleArgs::curry_tree_hash(base_price),
                start_premium: Self::get_start_premium(scale_factor),
                end_value: Self::get_end_value(scale_factor),
                precision: PREMIUM_PRECISION,
                bits_list: PREMIUM_BITS_LIST.to_vec(),
            },
        }
        .tree_hash()
    }

    pub fn get_puzzle(self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.xchandles_exponential_premium_renew_puzzle()?,
            args: self,
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_price(
        self,
        ctx: &mut SpendContext,
        handle: String,
        expiration: u64,
        buy_time: u64,
        num_years: u64,
    ) -> Result<u128, DriverError> {
        let puzzle = self.get_puzzle(ctx)?;
        let solution =
            XchandlesExponentialPremiumRenewPuzzleSolution::<XchandlesFactorPricingSolution> {
                buy_time,
                pricing_program_solution: XchandlesFactorPricingSolution {
                    current_expiration: expiration,
                    handle,
                    num_years,
                },
            }
            .to_clvm(&mut ctx.allocator)?;
        let output = ctx.run(puzzle, solution)?;

        Ok(<(u128, u64)>::from_clvm(&ctx.allocator, output)?.0)
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesExponentialPremiumRenewPuzzleSolution<S> {
    pub buy_time: u64,
    #[clvm(rest)]
    pub pricing_program_solution: S,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
    #[clvm(list)]
    pub struct XchandlesPricingOutput {
        pub price: u128,
        #[clvm(rest)]
        pub registered_time: u64,
    }

    #[test]
    fn test_exponential_premium_puzzle() -> Result<(), DriverError> {
        let mut ctx = SpendContext::new();

        let puzzle =
            XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(&mut ctx, 0, 1000)?
                .get_puzzle(&mut ctx)?;

        let mut last_price = 100_000_000_000;
        for day in 0..28 {
            for hour in 0..24 {
                let buy_time = day * 24 * 60 * 60 + hour * 60 * 60;
                let solution = XchandlesExponentialPremiumRenewPuzzleSolution::<
                    XchandlesFactorPricingSolution,
                > {
                    buy_time,
                    pricing_program_solution: XchandlesFactorPricingSolution {
                        current_expiration: 0,
                        handle: "yakuhito".to_string(),
                        num_years: 1,
                    },
                }
                .to_clvm(&mut ctx.allocator)?;

                let output = ctx.run(puzzle, solution)?;
                let output = XchandlesPricingOutput::from_clvm(&ctx.allocator, output)?;

                assert_eq!(output.registered_time, 366 * 24 * 60 * 60);

                if hour == 0 {
                    let scale_factor =
                        372529029846191406_u128 * 1000_u128 / 1_000_000_000_000_000_000_u128;
                    assert_eq!(
                        output.price,
                        (100_000_000 * 1000) / (1 << day) - scale_factor
                    );
                }

                assert!(output.price < last_price);
                last_price = output.price;

                assert_eq!(
                    XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(
                        &mut ctx, 0, 1000
                    )?
                    .get_price(
                        &mut ctx,
                        "yakuhito".to_string(),
                        0,
                        buy_time,
                        1
                    )?,
                    output.price
                );
            }
        }

        // check premium after auction is 0
        let solution =
            XchandlesExponentialPremiumRenewPuzzleSolution::<XchandlesFactorPricingSolution> {
                buy_time: 28 * 24 * 60 * 60,
                pricing_program_solution: XchandlesFactorPricingSolution {
                    current_expiration: 0,
                    handle: "yakuhito".to_string(),
                    num_years: 1,
                },
            }
            .to_clvm(&mut ctx.allocator)?;

        let output = ctx.run(puzzle, solution)?;
        let output = XchandlesPricingOutput::from_clvm(&ctx.allocator, output)?;

        assert_eq!(output.registered_time, 366 * 24 * 60 * 60);
        assert_eq!(output.price, 0);

        Ok(())
    }
}
