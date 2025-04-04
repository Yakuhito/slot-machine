use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{
    driver::{DriverError, Spend, SpendContext},
    types::{announcement_id, Conditions},
};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, Slot, SpendContextExt, XchandlesConstants, XchandlesRegistry, XchandlesSlotValue,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesOracleAction {
    pub launcher_id: Bytes32,
}

impl ToTreeHash for XchandlesOracleAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesOracleActionArgs::curry_tree_hash(self.launcher_id)
    }
}

impl Action<XchandlesRegistry> for XchandlesOracleAction {
    fn from_constants(constants: &XchandlesConstants) -> Self {
        Self {
            launcher_id: constants.launcher_id,
        }
    }
}

impl XchandlesOracleAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_oracle_puzzle()?,
            args: XchandlesOracleActionArgs::new(self.launcher_id),
        }
        .to_clvm(ctx)?)
    }

    pub fn get_slot_value(&self, old_slot_value: XchandlesSlotValue) -> XchandlesSlotValue {
        old_slot_value // nothing changed
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        registry: &mut XchandlesRegistry,
        slot: Slot<XchandlesSlotValue>,
    ) -> Result<(Conditions, Slot<XchandlesSlotValue>), DriverError> {
        // spend slots
        slot.spend(ctx, registry.info.inner_puzzle_hash().into())?;

        // finally, spend self
        let action_solution = ctx.alloc(&XchandlesOracleActionSolution {
            data_treehash: slot.info.value.tree_hash().into(),
        })?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        registry.insert(Spend::new(action_puzzle, action_solution));

        let new_slot = self.get_slot_value(slot.info.value);

        let mut oracle_ann = slot.info.value.tree_hash().to_vec();
        oracle_ann.insert(0, b'o');
        Ok((
            Conditions::new()
                .assert_puzzle_announcement(announcement_id(registry.coin.puzzle_hash, oracle_ann)),
            registry.created_slot_values_to_slots(vec![new_slot])[0],
        ))
    }
}

pub const XCHANDLES_ORACLE_PUZZLE: [u8; 521] = hex!("ff02ffff01ff04ff0bffff04ffff02ff3effff04ff02ffff04ff05ffff04ff27ff8080808080ffff04ffff02ff1affff04ff02ffff04ff05ffff04ff27ff8080808080ffff04ffff04ff18ffff04ffff0effff016fff2780ff808080ff8080808080ffff04ffff01ffffff333eff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff2effff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff04ff10ffff04ffff02ff16ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff52ffff02ff2effff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff16ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_ORACLE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    594aa7ec5ccc704bb182309b8b41b531103a12eca6baf3135b4a3b9ef8394a67
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesOracleActionArgs {
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesOracleActionArgs {
    pub fn new(launcher_id: Bytes32) -> Self {
        Self {
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id, 0).into(),
        }
    }
}

impl XchandlesOracleActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_ORACLE_PUZZLE_HASH,
            args: XchandlesOracleActionArgs::new(launcher_id),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesOracleActionSolution {
    pub data_treehash: Bytes32,
}
