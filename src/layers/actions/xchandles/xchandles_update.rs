use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::{SINGLETON_LAUNCHER_PUZZLE_HASH, SINGLETON_TOP_LAYER_PUZZLE_HASH},
};
use chia_wallet_sdk::{Conditions, DriverError, Spend, SpendContext};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, Slot, SpendContextExt, XchandlesConstants, XchandlesRegistry, XchandlesSlotValue,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesUpdateAction {
    pub launcher_id: Bytes32,
}

impl ToTreeHash for XchandlesUpdateAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesUpdateActionArgs::curry_tree_hash(self.launcher_id)
    }
}

impl Action<XchandlesRegistry> for XchandlesUpdateAction {
    fn from_constants(launcher_id: Bytes32, _constants: &XchandlesConstants) -> Self {
        Self { launcher_id }
    }
}

impl XchandlesUpdateAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_update_puzzle()?,
            args: XchandlesUpdateActionArgs::new(self.launcher_id),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    pub fn get_slot_value_from_solution(
        &self,
        ctx: &mut SpendContext,
        spent_slot_value: XchandlesSlotValue,
        solution: NodePtr,
    ) -> Result<XchandlesSlotValue, DriverError> {
        let solution = XchandlesUpdateActionSolution::from_clvm(&ctx.allocator, solution)?;

        Ok(spent_slot_value.with_launcher_ids(
            solution.new_owner_launcher_id,
            solution.new_resolved_launcher_id,
        ))
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        registry: &mut XchandlesRegistry,
        slot: Slot<XchandlesSlotValue>,
        new_owner_launcher_id: Bytes32,
        new_resolved_launcher_id: Bytes32,
        announcer_inner_puzzle_hash: Bytes32,
    ) -> Result<(Conditions, Slot<XchandlesSlotValue>), DriverError> {
        // spend slots
        let Some(slot_value) = slot.info.value else {
            return Err(DriverError::Custom("Missing slot value".to_string()));
        };

        let my_inner_puzzle_hash: Bytes32 = registry.info.inner_puzzle_hash().into();

        slot.spend(ctx, my_inner_puzzle_hash)?;

        // spend self
        let action_solution = XchandlesUpdateActionSolution {
            value_hash: slot_value.handle_hash.tree_hash().into(),
            neighbors_hash: slot_value.neighbors.tree_hash().into(),
            expiration: slot_value.expiration,
            current_owner_launcher_id: slot_value.owner_launcher_id,
            current_resolved_launcher_id: slot_value.resolved_launcher_id,
            new_owner_launcher_id,
            new_resolved_launcher_id,
            announcer_inner_puzzle_hash,
        }
        .to_clvm(&mut ctx.allocator)?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        registry.insert(Spend::new(action_puzzle, action_solution));

        let new_slot_value = self.get_slot_value_from_solution(ctx, slot_value, action_solution)?;

        let msg: Bytes32 = clvm_tuple!(
            slot_value.handle_hash,
            clvm_tuple!(new_owner_launcher_id, new_resolved_launcher_id)
        )
        .tree_hash()
        .into();
        Ok((
            Conditions::new().send_message(
                18,
                msg.into(),
                vec![ctx.alloc(&registry.coin.puzzle_hash)?],
            ),
            registry.created_slot_values_to_slots(vec![new_slot_value])[0],
        ))
    }
}

pub const XCHANDLES_UPDATE_PUZZLE: [u8; 806] = hex!("ff02ffff01ff04ff2fffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff02ff16ffff04ff02ffff04ff819fffff04ff82015fffff04ff8202dfffff04ff8205dfffff04ff820bdfff8080808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff17ffff04ffff02ff16ffff04ff02ffff04ff819fffff04ff82015fffff04ff8202dfffff04ff8217dfffff04ff822fdfff8080808080808080ff8080808080ffff04ffff04ff18ffff04ffff0112ffff04ffff0bffff0102ff819fffff0bffff0102ffff0bffff0101ff8217df80ffff0bffff0101ff822fdf808080ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0101ff8205df80ff0b8080ffff04ff823fdfff808080808080ff8080808080ff8080808080ffff04ffff01ffffff3343ff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff2effff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff10ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ff0bff52ffff02ff2effff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bffff0102ff05ffff0bffff0102ff0bffff0bffff0102ffff0bffff0101ff1780ffff0bffff0102ffff0bffff0101ff2f80ffff0bffff0101ff5f8080808080ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_UPDATE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    4d969f185cd1c8f3476c7627b0da5ba9d009b70975e6e8eccf222ae8d019aa5b
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesUpdateActionArgs {
    pub singleton_mod_hash: Bytes32,
    pub singleton_launcher_mod_hash_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesUpdateActionArgs {
    pub fn new(launcher_id: Bytes32) -> Self {
        let singleton_launcher_mod_hash: Bytes32 = SINGLETON_LAUNCHER_PUZZLE_HASH.into();
        Self {
            singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
            singleton_launcher_mod_hash_hash: singleton_launcher_mod_hash.tree_hash().into(),
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id, 0).into(),
        }
    }
}

impl XchandlesUpdateActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_UPDATE_PUZZLE_HASH,
            args: XchandlesUpdateActionArgs::new(launcher_id),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesUpdateActionSolution {
    pub value_hash: Bytes32,
    pub neighbors_hash: Bytes32,
    pub expiration: u64,
    pub current_owner_launcher_id: Bytes32,
    pub current_resolved_launcher_id: Bytes32,
    pub new_owner_launcher_id: Bytes32,
    pub new_resolved_launcher_id: Bytes32,
    #[clvm(rest)]
    pub announcer_inner_puzzle_hash: Bytes32,
}
