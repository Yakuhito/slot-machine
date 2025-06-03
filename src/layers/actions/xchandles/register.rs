use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
    sha2::Sha256,
};
use chia_wallet_sdk::{
    driver::{DriverError, Spend, SpendContext},
    types::{announcement_id, Conditions},
};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, DefaultCatMakerArgs, PrecommitCoin, PrecommitLayer, Slot, SpendContextExt,
    XchandlesConstants, XchandlesPrecommitValue, XchandlesRegistry, XchandlesSlotValue,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesRegisterAction {
    pub launcher_id: Bytes32,
    pub relative_block_height: u32,
    pub payout_puzzle_hash: Bytes32,
}

impl ToTreeHash for XchandlesRegisterAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesRegisterActionArgs::curry_tree_hash(
            self.launcher_id,
            self.relative_block_height,
            self.payout_puzzle_hash,
        )
    }
}

impl Action<XchandlesRegistry> for XchandlesRegisterAction {
    fn from_constants(constants: &XchandlesConstants) -> Self {
        Self {
            launcher_id: constants.launcher_id,
            relative_block_height: constants.relative_block_height,
            payout_puzzle_hash: constants.precommit_payout_puzzle_hash,
        }
    }
}

impl XchandlesRegisterAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_register_puzzle()?,
            args: XchandlesRegisterActionArgs::new(
                self.launcher_id,
                self.relative_block_height,
                self.payout_puzzle_hash,
            ),
        }
        .to_clvm(ctx)?)
    }

    pub fn get_spent_slot_value_hash(
        handle_hash: Bytes32,
        left_value: Bytes32,
        right_value: Bytes32,
        data_hash: Bytes32,
    ) -> Bytes32 {
        let mut hasher = Sha256::new();

        hasher.update(b"\x02");
        hasher.update(clvm_tuple!(left_value, right_value).tree_hash());
        hasher.update(data_hash);
        let after_handle_hash = hasher.finalize();

        hasher = Sha256::new();
        hasher.update(b"\x02");
        hasher.update(handle_hash.tree_hash());
        hasher.update(after_handle_hash);
        hasher.finalize().into()
    }

    pub fn get_spent_slot_value_hashes_from_solution(
        ctx: &SpendContext,
        solution: NodePtr,
    ) -> Result<[Bytes32; 2], DriverError> {
        let solution = XchandlesRegisterActionSolution::<
            NodePtr,
            XchandlesFactorPricingSolution,
            NodePtr,
            NodePtr,
        >::from_clvm(ctx, solution)?;

        Ok([
            Self::get_spent_slot_value_hash(
                solution.left_value,
                solution.left_left_value,
                solution.right_value,
                solution.left_data_hash,
            ),
            Self::get_spent_slot_value_hash(
                solution.right_value,
                solution.left_value,
                solution.right_right_value,
                solution.right_data_hash,
            ),
        ])
    }

    pub fn get_slot_values_from_solution(
        ctx: &mut SpendContext,
        spent_slot_values: [XchandlesSlotValue; 2],
        precommit_coin_value: XchandlesPrecommitValue,
        solution: NodePtr,
    ) -> Result<[XchandlesSlotValue; 3], DriverError> {
        let (left_slot_value, right_slot_value) = if spent_slot_values[0] < spent_slot_values[1] {
            (spent_slot_values[0], spent_slot_values[1])
        } else {
            (spent_slot_values[1], spent_slot_values[0])
        };

        let solution = XchandlesRegisterActionSolution::<
            NodePtr,
            XchandlesFactorPricingSolution,
            NodePtr,
            NodePtr,
        >::from_clvm(ctx, solution)?;

        Ok([
            left_slot_value
                .with_neighbors(left_slot_value.neighbors.left_value, solution.handle_hash),
            XchandlesSlotValue::new(
                solution.handle_hash,
                left_slot_value.handle_hash,
                right_slot_value.handle_hash,
                precommit_coin_value.start_time
                    + solution.pricing_puzzle_solution.num_years * 366 * 24 * 60 * 60,
                precommit_coin_value.owner_launcher_id,
                precommit_coin_value.resolved_launcher_id,
            ),
            right_slot_value
                .with_neighbors(solution.handle_hash, right_slot_value.neighbors.right_value),
        ])
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        registry: &mut XchandlesRegistry,
        left_slot: Slot<XchandlesSlotValue>,
        right_slot: Slot<XchandlesSlotValue>,
        precommit_coin: PrecommitCoin<XchandlesPrecommitValue>,
        base_handle_price: u64,
    ) -> Result<(Conditions, [Slot<XchandlesSlotValue>; 3]), DriverError> {
        // spend slots
        let my_inner_puzzle_hash: Bytes32 = registry.info.inner_puzzle_hash().into();

        registry
            .pending_items
            .spent_slots
            .push(left_slot.info.value_hash);
        registry
            .pending_items
            .spent_slots
            .push(right_slot.info.value_hash);

        left_slot.spend(ctx, my_inner_puzzle_hash)?;
        right_slot.spend(ctx, my_inner_puzzle_hash)?;

        let handle: String = precommit_coin.value.secret_and_handle.handle.clone();
        let handle_hash: Bytes32 = handle.tree_hash().into();

        let secret = precommit_coin.value.secret_and_handle.secret;

        let start_time = precommit_coin.value.start_time;

        let num_years = precommit_coin.coin.amount
            / XchandlesFactorPricingPuzzleArgs::get_price(base_handle_price, &handle, 1);
        let expiration = precommit_coin.value.start_time + num_years * 366 * 24 * 60 * 60;

        // calculate announcement
        let register_announcement: Bytes32 = clvm_tuple!(
            handle.clone(),
            clvm_tuple!(
                expiration,
                clvm_tuple!(
                    precommit_coin.value.owner_launcher_id,
                    precommit_coin.value.resolved_launcher_id
                )
            )
        )
        .tree_hash()
        .into();
        let mut register_announcement: Vec<u8> = register_announcement.to_vec();
        register_announcement.insert(0, b'r');

        // spend precommit coin
        precommit_coin.spend(
            ctx,
            1, // mode 1 = register/expire (use value)
            my_inner_puzzle_hash,
        )?;

        // finally, spend self
        let action_solution = XchandlesRegisterActionSolution {
            handle_hash,
            left_value: left_slot.info.value.handle_hash,
            right_value: right_slot.info.value.handle_hash,
            pricing_puzzle_reveal: XchandlesFactorPricingPuzzleArgs::get_puzzle(
                ctx,
                base_handle_price,
            )?,
            pricing_puzzle_solution: XchandlesFactorPricingSolution {
                current_expiration: 0,
                handle: handle.clone(),
                num_years,
            },
            cat_maker_reveal: DefaultCatMakerArgs::get_puzzle(
                ctx,
                precommit_coin.asset_id.tree_hash().into(),
            )?,
            cat_maker_solution: (),
            rest_data_hash: clvm_tuple!(
                precommit_coin.value.owner_launcher_id,
                precommit_coin.value.resolved_launcher_id
            )
            .tree_hash()
            .into(),
            start_time,
            secret_hash: secret.tree_hash().into(),
            refund_puzzle_hash_hash: precommit_coin.refund_puzzle_hash.tree_hash().into(),
            left_left_value: left_slot.info.value.neighbors.left_value,
            left_data_hash: left_slot.info.value.after_neigbors_data_hash().into(),
            right_right_value: right_slot.info.value.neighbors.right_value,
            right_data_hash: right_slot.info.value.after_neigbors_data_hash().into(),
        }
        .to_clvm(ctx)?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        registry.insert(Spend::new(action_puzzle, action_solution));

        let new_slots_values = Self::get_slot_values_from_solution(
            ctx,
            [left_slot.info.value, right_slot.info.value],
            precommit_coin.value,
            action_solution,
        )?;

        registry.pending_items.slot_values.push(new_slots_values[0]);
        registry.pending_items.slot_values.push(new_slots_values[1]);
        registry.pending_items.slot_values.push(new_slots_values[2]);

        Ok((
            Conditions::new().assert_puzzle_announcement(announcement_id(
                registry.coin.puzzle_hash,
                register_announcement,
            )),
            registry
                .created_slot_values_to_slots(new_slots_values.to_vec())
                .try_into()
                .unwrap(),
        ))
    }
}

pub const XCHANDLES_REGISTER_PUZZLE: [u8; 1487] = hex!("ff02ffff01ff02ffff03ffff22ffff09ff4fffff0bffff0101ff8215ef8080ffff20ff8209ef80ffff0aff4fff81af80ffff0aff82016fff4f80ffff09ff57ffff02ff2effff04ff02ffff04ff820befff8080808080ffff09ff81b7ffff02ff2effff04ff02ffff04ff8202efff808080808080ffff01ff04ff17ffff02ff16ffff04ff02ffff04ff05ffff04ff0bffff04ff37ffff04ff8207efffff04ff4fffff04ff81afffff04ff82016fffff04ffff02ff8202efff8205ef80ffff04ffff02ff2effff04ff02ffff04ff8205efff80808080ff80808080808080808080808080ffff01ff088080ff0180ffff04ffff01ffffff5133ff3eff4202ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff18ffff04ffff0bff52ffff0bff3cffff0bff3cff62ff0580ffff0bff3cffff0bff72ffff0bff3cffff0bff3cff62ffff0bffff0101ff0b8080ffff0bff3cff62ff42808080ff42808080ffff04ff80ffff04ffff04ff17ff8080ff8080808080ff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0102ffff0bffff0101ff0b80ffff0bffff0101ff178080ff2f8080ffff04ffff04ff10ffff04ff8202efff808080ffff04ffff02ff3effff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff81bfffff04ff8217efffff04ff82017fffff04ff822fefff80808080808080ff8080808080ffff04ffff02ff3effff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff82017fffff04ff81bfffff04ff825fefffff04ff82bfefff80808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff5fffff04ff81bfffff04ff82017fffff04ffff0bffff0102ffff0bffff0101ffff10ff8202efff8206ff8080ff82016f80ff80808080808080ffff04ff5fff808080808080ffff04ffff02ff2affff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff81bfffff04ff8217efffff04ff5fffff04ff822fefff80808080808080ffff04ff81bfff808080808080ffff04ffff02ff2affff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff82017fffff04ff5fffff04ff825fefffff04ff82bfefff80808080808080ffff04ff82017fff808080808080ffff04ffff04ff14ffff04ffff0effff0172ffff0bffff0102ff5fffff0bffff0102ffff0bffff0101ffff10ff8206ffff8202ef8080ff82016f808080ff808080ffff04ffff04ff2cffff04ffff0113ffff04ffff0101ffff04ffff02ff4fffff04ffff0bff52ffff0bff3cffff0bff3cff62ff0580ffff0bff3cffff0bff72ffff0bff3cffff0bff3cff62ff820bef80ffff0bff3cffff0bff72ffff0bff3cffff0bff3cff62ffff0bffff0102ffff0bffff0101ffff0bffff0102ffff0bffff0102ff27ffff02ff2effff04ff02ffff04ff81afff8080808080ffff0bffff0102ff57ff8205ff808080ffff0bffff0102ffff0bffff0102ff8205efff5f80ffff0bffff0102ffff0bffff0101ff8202ef80ff82016f80808080ffff0bff3cff62ff42808080ff42808080ff42808080ffff04ff81afff80808080ffff04ff8204ffff808080808080ff808080808080808080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff2cffff04ffff0112ffff04ff80ffff04ffff0bff52ffff0bff3cffff0bff3cff62ff0580ffff0bff3cffff0bff72ffff0bff3cffff0bff3cff62ffff0bffff0101ff0b8080ffff0bff3cff62ff42808080ff42808080ff8080808080ff018080");

pub const XCHANDLES_REGISTER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    0e381c74a42618a49f6ab629c34faf20dc7d95c22bbfe7746c2495a9b86abc63
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesRegisterActionArgs {
    pub precommit_1st_curry_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesRegisterActionArgs {
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
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id, 0).into(),
        }
    }
}

impl XchandlesRegisterActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_REGISTER_PUZZLE_HASH,
            args: XchandlesRegisterActionArgs::new(
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
pub struct XchandlesRegisterActionSolution<PP, PS, CMP, CMS> {
    pub handle_hash: Bytes32,
    pub left_value: Bytes32,
    pub right_value: Bytes32,
    pub pricing_puzzle_reveal: PP,
    pub pricing_puzzle_solution: PS,
    pub cat_maker_reveal: CMP,
    pub cat_maker_solution: CMS,
    pub rest_data_hash: Bytes32,
    pub start_time: u64,
    pub secret_hash: Bytes32,
    pub refund_puzzle_hash_hash: Bytes32,
    pub left_left_value: Bytes32,
    pub left_data_hash: Bytes32,
    pub right_right_value: Bytes32,
    pub right_data_hash: Bytes32,
}

pub const XCHANDLES_FACTOR_PRICING_PUZZLE: [u8; 481] = hex!("ff02ffff01ff02ffff03ffff15ff1fff8080ffff01ff04ffff12ff1fff05ffff02ff0effff04ff02ffff04ffff0dff1780ffff04ffff02ff0affff04ff02ffff04ff17ff80808080ff808080808080ffff12ff1fff048080ffff01ff088080ff0180ffff04ffff01ff8401e28500ffff02ffff03ff05ffff01ff02ffff03ffff22ffff15ffff0cff05ff80ffff010180ffff016080ffff15ffff017bffff0cff05ff80ffff0101808080ffff01ff02ff0affff04ff02ffff04ffff0cff05ffff010180ff80808080ffff01ff02ffff03ffff22ffff15ffff0cff05ff80ffff010180ffff012f80ffff15ffff013affff0cff05ff80ffff0101808080ffff01ff10ffff0101ffff02ff0affff04ff02ffff04ffff0cff05ffff010180ff8080808080ffff01ff088080ff018080ff0180ff8080ff0180ff05ffff14ffff02ffff03ffff15ff05ffff010280ffff01ff02ffff03ffff15ff05ffff010480ffff01ff02ffff03ffff09ff05ffff010580ffff01ff0110ffff01ff02ffff03ffff15ff05ffff011f80ffff01ff0880ffff01ff010280ff018080ff0180ffff01ff02ffff03ffff09ff05ffff010380ffff01ff01820080ffff01ff014080ff018080ff0180ffff01ff088080ff0180ffff03ff0bffff0102ffff0101808080ff018080");

pub const XCHANDLES_FACTOR_PRICING_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    129896065abb6e13cce6f46c784add16c771336cfa39a5647644a95a0ee0abd7
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesFactorPricingPuzzleArgs {
    pub base_price: u64,
}

impl XchandlesFactorPricingPuzzleArgs {
    pub fn new(base_price: u64) -> Self {
        Self { base_price }
    }

    pub fn get_puzzle(ctx: &mut SpendContext, base_price: u64) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.xchandles_factor_pricing_puzzle()?,
            args: XchandlesFactorPricingPuzzleArgs::new(base_price),
        }
        .to_clvm(ctx)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_price(base_price: u64, handle: &str, num_years: u64) -> u64 {
        base_price
            * match handle.len() {
                3 => 128,
                4 => 64,
                5 => 16,
                _ => 2,
            }
            / if handle.contains(|c: char| c.is_numeric()) {
                2
            } else {
                1
            }
            * num_years
    }
}

impl XchandlesFactorPricingPuzzleArgs {
    pub fn curry_tree_hash(base_price: u64) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_FACTOR_PRICING_PUZZLE_HASH,
            args: XchandlesFactorPricingPuzzleArgs::new(base_price),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesFactorPricingSolution {
    pub current_expiration: u64,
    pub handle: String,
    #[clvm(rest)]
    pub num_years: u64,
}

#[cfg(test)]
mod tests {
    use clvmr::reduction::EvalErr;

    use super::*;

    #[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
    #[clvm(list)]
    pub struct XchandlesFactorPricingOutput {
        pub price: u64,
        #[clvm(rest)]
        pub registered_time: u64,
    }

    #[test]
    fn test_factor_pricing_puzzle() -> Result<(), DriverError> {
        let mut ctx = SpendContext::new();
        let base_price = 1; // puzzle will only spit out factors

        let puzzle = XchandlesFactorPricingPuzzleArgs::get_puzzle(&mut ctx, base_price)?;

        for handle_length in 3..=31 {
            for num_years in 1..=3 {
                for has_number in [false, true] {
                    let handle = if has_number {
                        "a".repeat(handle_length - 1) + "1"
                    } else {
                        "a".repeat(handle_length)
                    };

                    let solution = ctx.alloc(&XchandlesFactorPricingSolution {
                        current_expiration: (handle_length - 3) as u64, // shouldn't matter
                        handle,
                        num_years,
                    })?;

                    let output = ctx.run(puzzle, solution)?;
                    let output = ctx.extract::<XchandlesFactorPricingOutput>(output)?;

                    let mut expected_price = if handle_length == 3 {
                        128
                    } else if handle_length == 4 {
                        64
                    } else if handle_length == 5 {
                        16
                    } else {
                        2
                    };
                    if has_number {
                        expected_price /= 2;
                    }
                    expected_price *= num_years;

                    assert_eq!(output.price, expected_price);
                    assert_eq!(output.registered_time, num_years * 366 * 24 * 60 * 60);
                }
            }
        }

        // make sure the puzzle won't let us register a handle of length 2

        let solution = ctx.alloc(&XchandlesFactorPricingSolution {
            current_expiration: 0,
            handle: "aa".to_string(),
            num_years: 1,
        })?;

        let Err(DriverError::Eval(EvalErr(_, s))) = ctx.run(puzzle, solution) else {
            panic!("Expected error");
        };
        assert_eq!(s, "clvm raise");

        // make sure the puzzle won't let us register a handle of length 32

        let solution = ctx.alloc(&XchandlesFactorPricingSolution {
            current_expiration: 0,
            handle: "a".repeat(32),
            num_years: 1,
        })?;

        let Err(DriverError::Eval(EvalErr(_, s))) = ctx.run(puzzle, solution) else {
            panic!("Expected error");
        };
        assert_eq!(s, "clvm raise");

        // make sure the puzzle won't let us register a handle with invalid characters

        let solution = ctx.alloc(&XchandlesFactorPricingSolution {
            current_expiration: 0,
            handle: "yak@test".to_string(),
            num_years: 1,
        })?;

        let Err(DriverError::Eval(EvalErr(_, s))) = ctx.run(puzzle, solution) else {
            panic!("Expected error");
        };
        assert_eq!(s, "clvm raise");

        Ok(())
    }
}
