mod action_layer;
mod actions;

use chia_wallet_sdk::{DriverError, SpendContext};
use clvmr::NodePtr;

pub use action_layer::*;
pub use actions::*;

pub trait SpendContextExt {
    fn action_layer_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn delegated_state_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
}

impl SpendContextExt for SpendContext {
    /// Allocate the action layer puzzle and return its pointer.
    fn action_layer_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(ACTION_LAYER_PUZZLE_HASH, &ACTION_LAYER_PUZZLE)
    }
    
    /// Allocate the delegated state action puzzle and return its pointer.
    fn delegated_state_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(DELEGATED_STATE_ACTION_PUZZLE_HASH, &DELEGATED_STATE_ACTION_PUZZLE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chia::clvm_utils::tree_hash;

    // we really have to expose this in chia-sdk-test
    macro_rules! assert_puzzle_hash {
        ($puzzle:ident => $puzzle_hash:ident) => {
            let mut a = clvmr::Allocator::new();
            let ptr = clvmr::serde::node_from_bytes(&mut a, &$puzzle)?;
            let hash = tree_hash(&mut a, ptr);
            assert_eq!($puzzle_hash, hash);
        };
    }

    #[test]
    fn test_puzzle_hashes() -> anyhow::Result<()> {
        assert_puzzle_hash!(ACTION_LAYER_PUZZLE => ACTION_LAYER_PUZZLE_HASH);
        assert_puzzle_hash!(DELEGATED_STATE_ACTION_PUZZLE => DELEGATED_STATE_ACTION_PUZZLE_HASH);

        Ok(())
    }
}