use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::{SingletonStruct, SINGLETON_TOP_LAYER_PUZZLE_HASH},
};
use chia_wallet_sdk::{DriverError, Layer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{DigRewardDistributorInfo, DigSlotNonce, Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigInitiatePayoutAction {
    pub launcher_id: Bytes32,
    pub validator_launcher_id: Bytes32,
    pub payout_threshold: u64,
}

impl DigInitiatePayoutAction {
    pub fn from_info(info: &DigRewardDistributorInfo) -> Self {
        Self {
            launcher_id: info.launcher_id,
            validator_launcher_id: info.constants.validator_launcher_id,
            payout_threshold: info.constants.payout_threshold,
        }
    }
}

impl Layer for DigInitiatePayoutAction {
    type Solution = DigInitiatePayoutActionSolution;

    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_initiate_payout_action_puzzle()?,
            args: DigInitiatePayoutActionArgs::new(
                self.launcher_id,
                self.validator_launcher_id,
                self.payout_threshold,
            ),
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: DigInitiatePayoutActionSolution,
    ) -> Result<NodePtr, DriverError> {
        solution
            .to_clvm(&mut ctx.allocator)
            .map_err(DriverError::ToClvm)
    }

    fn parse_puzzle(
        _: &clvmr::Allocator,
        _: chia_wallet_sdk::Puzzle,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        unimplemented!()
    }

    fn parse_solution(_: &clvmr::Allocator, _: NodePtr) -> Result<Self::Solution, DriverError> {
        unimplemented!()
    }
}

impl DigInitiatePayoutAction {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        validator_launcher_id: Bytes32,
        payout_threshold: u64,
    ) -> TreeHash {
        CurriedProgram {
            program: DIG_INITIATE_PAYOUT_PUZZLE_HASH,
            args: DigInitiatePayoutActionArgs::new(
                launcher_id,
                validator_launcher_id,
                payout_threshold,
            ),
        }
        .tree_hash()
    }
}

pub const DIG_INITIATE_PAYOUT_PUZZLE: [u8; 790] = hex!("ff02ffff01ff04ffff04ffff11ff819fffff12ffff11ff8204dfff8202bf80ff8203bf8080ffff04ff82015fffff04ff8202dfffff04ff8205dfff8080808080ffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff02ff3affff04ff02ffff04ff82013fffff04ff8202bfffff04ff8203bfff808080808080ff8080808080ffff04ffff02ff12ffff04ff02ffff04ff17ffff04ffff02ff3affff04ff02ffff04ff82013fffff04ff8204dfffff04ff8203bfff808080808080ff8080808080ffff04ffff04ffff0181d6ffff04ff10ffff04ff82013fffff04ffff02ff2effff04ff02ffff04ffff12ffff11ff8204dfff8202bf80ff8203bf80ffff04ff2fff8080808080ffff04ffff04ff82013fff8080ff808080808080ff8080808080ffff04ffff01ffffff3342ff02ffff02ffff03ff05ffff01ff0bff81fcffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff2cffff04ff02ffff04ff0dff80808080ff808080808080ffff0181dc80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff04ff10ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff81bcffff02ff16ffff04ff02ffff04ff05ffff04ffff02ff2cffff04ff02ffff04ff07ff80808080ff808080808080ff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0101ff0b80ffff0bffff0101ff17808080ffff0bff14ffff0bff14ff81dcff0580ffff0bff14ff0bff819c8080ffff02ffff03ffff15ff05ff0b80ffff0105ffff01ff088080ff0180ff04ff18ffff04ffff0112ffff04ff80ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const DIG_INITIATE_PAYOUT_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    a6c49aefa34100f0b469d764aa6c33ce9a7c6de69138cca081b8ccea458cb69a
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigInitiatePayoutActionArgs {
    pub singleton_mod_hash: Bytes32,
    pub validator_singleton_struct_hash: Bytes32,
    pub mirror_slot_1st_curry_hash: Bytes32,
    pub payout_threshold: u64,
}

impl DigInitiatePayoutActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        validator_launcher_id: Bytes32,
        payout_threshold: u64,
    ) -> Self {
        Self {
            singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
            validator_singleton_struct_hash: SingletonStruct::new(validator_launcher_id)
                .tree_hash()
                .into(),
            mirror_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                Some(DigSlotNonce::MIRROR.to_u64()),
            )
            .into(),
            payout_threshold,
        }
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigInitiatePayoutActionSolution {
    pub mirror_payout_puzzle_hash: Bytes32,
    pub mirror_initial_cumulative_payout: u64,
    #[clvm(rest)]
    pub mirror_shares: u64,
}
