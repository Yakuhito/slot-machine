use chia_wallet_sdk::{DriverError, SpendContext};
use clvmr::NodePtr;

mod cat_nft_metadata;
mod cli;
mod debug;
mod drivers;
mod layers;
mod primitives;

pub use cat_nft_metadata::*;
pub use cli::*;
pub use debug::*;
pub use drivers::*;
pub use layers::*;
pub use primitives::*;

pub trait SpendContextExt {
    fn default_finalizer_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn action_layer_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn delegated_state_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn catalog_register_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn uniqueness_prelauncher_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn precommit_layer_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn slot_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn any_metadata_updater(&mut self) -> Result<NodePtr, DriverError>;
    fn verification_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn reserve_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_register_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_update_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_extend_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_expire_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_oracle_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn verification_payments_puzzle(&mut self) -> Result<NodePtr, DriverError>;
}

impl SpendContextExt for SpendContext {
    /// Allocate thedefault finalizer puzzle and return its pointer.
    fn default_finalizer_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(DEFAULT_FINALIZER_PUZZLE_HASH, &DEFAULT_FINALIZER_PUZZLE)
    }

    /// Allocate the action layer puzzle and return its pointer.
    fn action_layer_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(ACTION_LAYER_PUZZLE_HASH, &ACTION_LAYER_PUZZLE)
    }

    /// Allocate the delegated state action puzzle and return its pointer.
    fn delegated_state_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            DELEGATED_STATE_ACTION_PUZZLE_HASH,
            &DELEGATED_STATE_ACTION_PUZZLE,
        )
    }

    /// Allocate the catalog register action puzzle and return its pointer.
    fn catalog_register_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(CATALOG_REGISTER_PUZZLE_HASH, &CATALOG_REGISTER_PUZZLE)
    }

    /// Allocate the uniqueness prelauncher puzzle and return its pointer.
    fn uniqueness_prelauncher_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            UNIQUENESS_PRELAUNCHER_PUZZLE_HASH,
            &UNIQUENESS_PRELAUNCHER_PUZZLE,
        )
    }

    /// Allocate the precommit coin puzzle and return its pointer.
    fn precommit_layer_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(PRECOMMIT_LAYER_PUZZLE_HASH, &PRECOMMIT_LAYER_PUZZLE)
    }

    /// Allocate the slot puzzle and return its pointer.
    fn slot_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(SLOT_PUZZLE_HASH, &SLOT_PUZZLE)
    }

    /// Allocate the any metadata updater puzzle and return its pointer.
    fn any_metadata_updater(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(ANY_METADATA_UPDATER_HASH, &ANY_METADATA_UPDATER)
    }

    /// Allocate the verification puzzle and return its pointer.
    fn verification_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(VERIFICATION_LAYER_PUZZLE_HASH, &VERIFICATION_LAYER_PUZZLE)
    }

    /// Allocate the reserve puzzle and return its pointer.
    fn reserve_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(RESERVE_PUZZLE_HASH, &RESERVE_PUZZLE)
    }

    /// Allocate the XCHandles register puzzle and return its pointer.
    fn xchandles_register_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(XCHANDLES_REGISTER_PUZZLE_HASH, &XCHANDLES_REGISTER_PUZZLE)
    }

    /// Allocate the XCHandles update puzzle and return its pointer.
    fn xchandles_update_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(XCHANDLES_UPDATE_PUZZLE_HASH, &XCHANDLES_UPDATE_PUZZLE)
    }

    /// Allocate the XCHandles extend puzzle and return its pointer.
    fn xchandles_extend_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(XCHANDLES_EXTEND_PUZZLE_HASH, &XCHANDLES_EXTEND_PUZZLE)
    }

    /// Allocate the XCHandles expire puzzle and return its pointer.
    fn xchandles_expire_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(XCHANDLES_EXPIRE_PUZZLE_HASH, &XCHANDLES_EXPIRE_PUZZLE)
    }

    /// Allocate the XCHandles oracle puzzle and return its pointer.
    fn xchandles_oracle_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(XCHANDLES_ORACLE_PUZZLE_HASH, &XCHANDLES_ORACLE_PUZZLE)
    }

    /// Allocate the verification payments puzzle and return its pointer.
    fn verification_payments_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            VERIFICATION_PAYMENTS_PUZZLE_HASH,
            &VERIFICATION_PAYMENTS_PUZZLE,
        )
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
        assert_puzzle_hash!(DEFAULT_FINALIZER_PUZZLE => DEFAULT_FINALIZER_PUZZLE_HASH);
        assert_puzzle_hash!(ACTION_LAYER_PUZZLE => ACTION_LAYER_PUZZLE_HASH);
        assert_puzzle_hash!(DELEGATED_STATE_ACTION_PUZZLE => DELEGATED_STATE_ACTION_PUZZLE_HASH);
        assert_puzzle_hash!(CATALOG_REGISTER_PUZZLE => CATALOG_REGISTER_PUZZLE_HASH);
        assert_puzzle_hash!(UNIQUENESS_PRELAUNCHER_PUZZLE => UNIQUENESS_PRELAUNCHER_PUZZLE_HASH);
        assert_puzzle_hash!(PRECOMMIT_LAYER_PUZZLE => PRECOMMIT_LAYER_PUZZLE_HASH);
        assert_puzzle_hash!(SLOT_PUZZLE => SLOT_PUZZLE_HASH);
        assert_puzzle_hash!(ANY_METADATA_UPDATER => ANY_METADATA_UPDATER_HASH);
        assert_puzzle_hash!(VERIFICATION_LAYER_PUZZLE => VERIFICATION_LAYER_PUZZLE_HASH);
        assert_puzzle_hash!(RESERVE_PUZZLE => RESERVE_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_REGISTER_PUZZLE => XCHANDLES_REGISTER_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_UPDATE_PUZZLE => XCHANDLES_UPDATE_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_EXTEND_PUZZLE => XCHANDLES_EXTEND_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_EXPIRE_PUZZLE => XCHANDLES_EXPIRE_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_ORACLE_PUZZLE => XCHANDLES_ORACLE_PUZZLE_HASH);
        assert_puzzle_hash!(VERIFICATION_PAYMENTS_PUZZLE => VERIFICATION_PAYMENTS_PUZZLE_HASH);

        Ok(())
    }
}
