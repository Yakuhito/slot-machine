use chia::{clvm_traits::ToClvm, clvm_utils::TreeHash};
use chia_wallet_sdk::{Layer, SpendContext};
use clvmr::Allocator;

pub trait SingletonAction {
    fn puzzle_hash(&self, ctx: &mut SpendContext) -> TreeHash;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingletonActionRun<A, S> where A: SingletonAction {
    pub action: A,
    pub solution: S,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingletonActionLayer<S, A> where A: SingletonAction {
    pub state: S,
    pub actions: Vec<A>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingletonActionLayerSolution<A, S> where A: SingletonAction {
    pub action_runs: Vec<SingletonActionRun<A, S>>,
}

impl<S, A> SingletonActionLayer<S, A> where A: SingletonAction {
    pub fn new(state: S, actions: Vec<A>) -> Self {
        Self {
            state,
            actions,
        }
    }
}

impl<S, A, N> Layer for SingletonActionLayer<S, A> where A: SingletonAction, N: ToClvm<Allocator> {
    type Solution = SingletonActionLayerSolution<A, N>;
}

pub const ACTION_LAYER_PUZZLE: [u8; 100] = hex!("ff02ffff01ff02ff3affff04ff02ffff04ff05ffff04ff0bffff04ff17ffff04ff80ffff04ff2fff8080808080808080ffff04ffff01ffffff3302ffff02ffff03ff05ffff01ff0bff81ecffff02ff12ffff04ff02ffff04ff09ffff04ffff02ff14ffff04ff02ffff04ff0dff80808080ff808080808080ffff0181cc80ff0180ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff0bff81acffff02ff12ffff04ff02ffff04ff05ffff04ffff02ff14ffff04ff02ffff04ff07ff80808080ff808080808080ffffff0bff18ffff0bff18ff81ccff0580ffff0bff18ff0bff818c8080ffff02ffff03ff0bffff01ff04ff13ffff02ff2affff04ff02ffff04ff1bffff04ff05ff808080808080ffff010580ff0180ff02ffff03ff5fffff01ff02ffff03ffff09ff0bffff02ff3effff04ff02ffff04ffff0bffff0101ffff02ff2effff04ff02ffff04ff82029fff8080808080ffff04ff82011fff808080808080ffff01ff02ff16ffff04ff02ffff04ff05ffff04ff0bffff04ffff02ff82029fffff04ff17ffff04ff82039fff80808080ffff04ff81dfffff04ff2fff8080808080808080ffff01ff088080ff0180ffff01ff04ffff04ff10ffff04ffff02ff3cffff04ff02ffff04ff05ffff04ffff0bffff0101ff0580ffff04ffff0bffff0101ff0b80ffff04ffff02ff2effff04ff02ffff04ff17ff80808080ff80808080808080ffff01ff01808080ff2f8080ff0180ffff02ff3affff04ff02ffff04ff05ffff04ff0bffff04ff27ffff04ffff02ff2affff04ff02ffff04ff5fffff04ff37ff8080808080ffff04ff2fff8080808080808080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff02ffff03ff1bffff01ff02ff3effff04ff02ffff04ffff02ffff03ffff18ffff0101ff1380ffff01ff0bffff0102ff2bff0580ffff01ff0bffff0102ff05ff2b8080ff0180ffff04ffff04ffff17ff13ffff0181ff80ff3b80ff8080808080ffff010580ff0180ff018080");