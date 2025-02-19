use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::offer::{NotarizedPayment, Payment, SETTLEMENT_PAYMENTS_PUZZLE_HASH},
};
use chia_wallet_sdk::{announcement_id, Conditions, DriverError, Spend, SpendContext};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, DefaultCatMakerArgs, Slot, SpendContextExt, XchandlesConstants, XchandlesRegistry,
    XchandlesSlotValue,
};

use super::{XchandlesFactorPricingPuzzleArgs, XchandlesFactorPricingSolution};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesExtendAction {
    pub launcher_id: Bytes32,
    pub payout_puzzle_hash: Bytes32,
}

impl ToTreeHash for XchandlesExtendAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesExtendActionArgs::curry_tree_hash(self.launcher_id, self.payout_puzzle_hash)
    }
}

impl Action<XchandlesRegistry> for XchandlesExtendAction {
    fn from_constants(launcher_id: Bytes32, constants: &XchandlesConstants) -> Self {
        Self {
            launcher_id,
            payout_puzzle_hash: constants.precommit_payout_puzzle_hash,
        }
    }
}

impl XchandlesExtendAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_extend_puzzle()?,
            args: XchandlesExtendActionArgs::new(self.launcher_id, self.payout_puzzle_hash),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    pub fn get_slot_value_from_solution(
        &self,
        ctx: &SpendContext,
        old_slot_value: XchandlesSlotValue,
        solution: NodePtr,
    ) -> Result<XchandlesSlotValue, DriverError> {
        let solution = XchandlesExtendActionSolution::<
            NodePtr,
            XchandlesFactorPricingSolution,
            NodePtr,
            (),
        >::from_clvm(&ctx.allocator, solution)?;

        Ok(old_slot_value.with_expiration(
            old_slot_value.expiration + solution.pricing_solution.num_years * 366 * 24 * 60 * 60,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        registry: &mut XchandlesRegistry,
        handle: String,
        slot: Slot<XchandlesSlotValue>,
        payment_asset_id: Bytes32,
        base_handle_price: u64,
        num_years: u64,
    ) -> Result<(NotarizedPayment, Conditions, Slot<XchandlesSlotValue>), DriverError> {
        // spend slots
        let Some(slot_value) = slot.info.value else {
            return Err(DriverError::Custom("Missing slot value".to_string()));
        };

        let spender_inner_puzzle_hash: Bytes32 = registry.info.inner_puzzle_hash().into();

        slot.spend(ctx, spender_inner_puzzle_hash)?;

        // finally, spend self
        let action_solution = XchandlesExtendActionSolution {
            handle_hash: slot_value.handle_hash,
            pricing_puzzle_reveal: XchandlesFactorPricingPuzzleArgs::get_puzzle(
                ctx,
                base_handle_price,
            )?,
            pricing_solution: XchandlesFactorPricingSolution {
                current_expiration: slot_value.expiration,
                handle: handle.clone(),
                num_years,
            },
            cat_maker_puzzle_reveal: DefaultCatMakerArgs::get_puzzle(
                ctx,
                payment_asset_id.tree_hash().into(),
            )?,
            cat_maker_solution: (),
            neighbors_hash: slot_value.neighbors.tree_hash().into(),
            expiration: slot_value.expiration,
            rest_hash: slot_value.launcher_ids_data_hash().into(),
        }
        .to_clvm(&mut ctx.allocator)?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        registry.insert(Spend::new(action_puzzle, action_solution));

        let renew_amount =
            XchandlesFactorPricingPuzzleArgs::get_price(base_handle_price, &handle, num_years);

        let notarized_payment = NotarizedPayment {
            nonce: clvm_tuple!(handle.clone(), slot_value.expiration)
                .tree_hash()
                .into(),
            payments: vec![Payment {
                puzzle_hash: registry.info.constants.precommit_payout_puzzle_hash,
                amount: renew_amount,
                memos: None,
            }],
        };

        let mut extend_ann: Vec<u8> = clvm_tuple!(renew_amount, handle).tree_hash().to_vec();
        extend_ann.insert(0, b'e');

        let new_slot_value = self.get_slot_value_from_solution(ctx, slot_value, action_solution)?;

        Ok((
            notarized_payment,
            Conditions::new()
                .assert_puzzle_announcement(announcement_id(registry.coin.puzzle_hash, extend_ann)),
            registry.created_slot_values_to_slots(vec![new_slot_value])[0],
        ))
    }
}

pub const XCHANDLES_EXTEND_PUZZLE: [u8; 1043] = hex!("ff02ffff01ff02ffff03ffff22ffff09ff4fffff02ff2effff04ff02ffff04ff8205dfff8080808080ffff09ff81afffff02ff2effff04ff02ffff04ff82015fff8080808080ffff09ff8204dfff822fdf80ffff09ff819fffff0bffff0101ff820adf808080ffff01ff04ff2fffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff819fffff04ff8217dfffff04ff822fdfffff04ff823fdfff80808080808080ff8080808080ffff04ffff04ff24ffff04ffff0effff0165ffff0bffff0102ffff0bffff0101ffff05ffff02ff82015fff8202df808080ff819f8080ff808080ffff04ffff04ff10ffff04ff822fdfff808080ffff04ffff02ff2affff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff819fffff04ff8217dfffff04ffff10ff822fdfffff06ffff02ff82015fff8202df808080ffff04ff823fdfff80808080808080ff8080808080ffff04ffff04ff28ffff04ffff0bffff02ff8205dfffff04ff05ffff04ff820bdfff80808080ffff02ff2effff04ff02ffff04ffff04ffff0bffff0102ff819fffff0bffff0101ff822fdf8080ffff04ffff04ff0bffff04ffff05ffff02ff82015fff8202df8080ff808080ff808080ff8080808080ff808080ff80808080808080ffff01ff088080ff0180ffff04ffff01ffffff55ff3f33ffff3e42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff36ffff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff38ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ff0bff52ffff02ff36ffff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ff0bffff0bffff0102ffff0bffff0101ff1780ff2f808080ff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff34ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_EXTEND_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    0ee45f0f32b1bcc836cf2063aa71126d6d58ec19c165de12bdc74e0292328f85
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesExtendActionArgs {
    pub offer_mod_hash: Bytes32,
    pub payout_puzzle_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesExtendActionArgs {
    pub fn new(launcher_id: Bytes32, payout_puzzle_hash: Bytes32) -> Self {
        Self {
            offer_mod_hash: SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
            payout_puzzle_hash,
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id, 0).into(),
        }
    }
}

impl XchandlesExtendActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32, payout_puzzle_hash: Bytes32) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_EXTEND_PUZZLE_HASH,
            args: XchandlesExtendActionArgs::new(launcher_id, payout_puzzle_hash),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesExtendActionSolution<PP, PS, CMP, CMS> {
    pub handle_hash: Bytes32,
    pub pricing_puzzle_reveal: PP,
    pub pricing_solution: PS,
    pub cat_maker_puzzle_reveal: CMP,
    pub cat_maker_solution: CMS,
    pub neighbors_hash: Bytes32,
    pub expiration: u64,
    #[clvm(rest)]
    pub rest_hash: Bytes32,
}
