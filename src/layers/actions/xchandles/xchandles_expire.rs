use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{Slot, SpendContextExt};

use super::XchandlesFactorPricingPuzzleArgs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesExpireAction {
    pub launcher_id: Bytes32,
}

impl XchandlesExpireAction {
    pub fn new(launcher_id: Bytes32) -> Self {
        Self { launcher_id }
    }
}

impl Layer for XchandlesExpireAction {
    type Solution = XchandlesExpireActionSolution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_expire_puzzle()?,
            args: XchandlesExpireActionArgs::new(self.launcher_id),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: XchandlesExpireActionSolution,
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

impl ToTreeHash for XchandlesExpireAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesExpireActionArgs::curry_tree_hash(self.launcher_id)
    }
}

pub const XCHANDLES_EXPIRE_PUZZLE: [u8; 913] = hex!("ff02ffff01ff04ff0bffff04ffff04ff10ffff04ff8217f7ff808080ffff04ffff04ff38ffff04ffff0effff0178ff2780ff808080ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff27ffff04ff57ffff04ff8202f7ffff04ffff0bffff0102ffff0bffff0101ff8217f780ff821ff780ff80808080808080ff8080808080ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff57ffff04ff81b7ffff04ff27ffff04ff820177ff80808080808080ff8080808080ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff8202f7ffff04ff27ffff04ff8205f7ffff04ff820bf7ff80808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff57ffff04ff81b7ffff04ff8202f7ffff04ff820177ff80808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff8202f7ffff04ff57ffff04ff8205f7ffff04ff820bf7ff80808080808080ff8080808080ff808080808080808080ffff04ffff01ffffff51ff333eff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff2effff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff28ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ff0bff52ffff02ff2effff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0102ffff0bffff0101ff0b80ffff0bffff0101ff178080ff2f8080ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_EXPIRE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    b033c6af2d0c34961c8304af66c096f4fa8de0bb4bc30f3ab017cb26aa83532e
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesExpireActionArgs {
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesExpireActionArgs {
    pub fn new(launcher_id: Bytes32) -> Self {
        Self {
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id).into(),
        }
    }
}

impl XchandlesExpireActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_EXPIRE_PUZZLE_HASH,
            args: XchandlesExpireActionArgs::new(launcher_id),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesExpireActionSolution {
    pub value: Bytes32,
    pub left_value: Bytes32,
    pub left_left_value: Bytes32,
    pub left_rest_hash: Bytes32,
    pub right_value: Bytes32,
    pub right_right_value: Bytes32,
    pub right_rest_hash: Bytes32,
    pub expiration: u64,
    #[clvm(rest)]
    pub launcher_id_hash: Bytes32,
}

pub const XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE: [u8; 305] = hex!("ff02ffff01ff04ffff10ffff05ffff02ff05ffff04ff81bfffff04ff8205ffff8080808080ffff02ff06ffff04ff02ffff04ff2fffff04ff5fffff04ffff0101ffff04ffff3dffff12ffff0183010000ffff11ff8202ffff82017f8080ff0480ffff04ffff05ffff14ff0bffff17ffff0102ffff05ffff14ffff11ff8202ffff82017f80ff048080808080ff808080808080808080ffff06ffff02ff05ffff04ff81bfffff04ff8205ffff808080808080ffff04ffff01ff83015180ff02ffff03ff0bffff01ff02ff06ffff04ff02ffff04ff05ffff04ff1bffff04ffff17ff17ffff010180ffff04ff2fffff04ffff10ff5fffff02ffff03ffff18ff2fff1780ffff01ff05ffff14ffff12ff5fff1380ff058080ff8080ff018080ff8080808080808080ffff015f80ff0180ff018080");

pub const XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    b033c6af2d0c34961c8304af66c096f4fa8de0bb4bc30f3ab017cb26aa83532e
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

impl XchandlesExponentialPremiumRenewPuzzleArgs<NodePtr> {
    // A scale factor is how many units of the payment token equate to $1
    // For exampe, you'd use scale_factor=1000 for wUSDC.b
    pub fn from_scale_factor(
        ctx: &mut SpendContext,
        base_price: u64,
        scale_factor: u64,
    ) -> Result<Self, DriverError> {
        Ok(Self {
            base_program: XchandlesFactorPricingPuzzleArgs::new(base_price).get_puzzle(ctx)?,
            start_premium: 100000000 * scale_factor, // start auction at $100 million
            end_value: scale_factor,
            precision: 1000000000000000000, // 10^18
            // https://github.com/ensdomains/ens-contracts/blob/master/contracts/ethregistrar/ExponentialPremiumPriceOracle.sol
            bits_list: vec![
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
            ],
        })
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesExponentialPremiumRenewPuzzleSolution<S> {
    pub handle: String,
    pub expiration: u64,
    pub buy_time: u64,
    pub pricing_program_solution: S,
}
