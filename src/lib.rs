use chia_wallet_sdk::driver::{DriverError, SpendContext};
use clvmr::NodePtr;

mod cat_nft_metadata;
mod cli;
mod debug;
mod drivers;
mod layers;
mod name_nft_metadata;
mod primitives;

pub use cat_nft_metadata::*;
pub use cli::*;
pub use debug::*;
pub use drivers::*;
pub use layers::*;
pub use name_nft_metadata::*;
pub use primitives::*;
pub trait SpendContextExt {
    fn default_finalizer_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn action_layer_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn delegated_state_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn catalog_register_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn catalog_refund_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn uniqueness_prelauncher_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn precommit_layer_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn slot_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn any_metadata_updater(&mut self) -> Result<NodePtr, DriverError>;
    fn verification_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_register_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_update_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_extend_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_expire_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_oracle_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_refund_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn verification_payments_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_factor_pricing_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn xchandles_exponential_premium_renew_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn default_cat_maker_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn dig_add_incentives_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn dig_add_mirror_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn dig_commit_incentives_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn dig_initiate_payout_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn dig_new_epoch_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn dig_remove_mirror_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn dig_sync_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn dig_withdraw_incentives_action_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn reserve_finalizer_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn p2_delegated_by_singleton_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn state_scheduler_puzzle(&mut self) -> Result<NodePtr, DriverError>;
    fn p2_m_of_n_delegate_direct_puzzle(&mut self) -> Result<NodePtr, DriverError>;
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

    /// Allocate the catalog refund action puzzle and return its pointer.
    fn catalog_refund_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(CATALOG_REFUND_PUZZLE_HASH, &CATALOG_REFUND_PUZZLE)
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

    /// Allocate the XCHandles refund puzzle and return its pointer.
    fn xchandles_refund_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(XCHANDLES_REFUND_PUZZLE_HASH, &XCHANDLES_REFUND_PUZZLE)
    }

    /// Allocate the verification payments puzzle and return its pointer.
    fn verification_payments_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            VERIFICATION_PAYMENTS_PUZZLE_HASH,
            &VERIFICATION_PAYMENTS_PUZZLE,
        )
    }

    /// Allocate the XCHandles factor pricing puzzle and return its pointer.
    fn xchandles_factor_pricing_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            XCHANDLES_FACTOR_PRICING_PUZZLE_HASH,
            &XCHANDLES_FACTOR_PRICING_PUZZLE,
        )
    }

    /// Allocate the XCHandles exponential premium renew puzzle and return its pointer.
    fn xchandles_exponential_premium_renew_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE_HASH,
            &XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE,
        )
    }

    /// Allocate the default CAT maker puzzle and return its pointer.
    fn default_cat_maker_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(DEFAULT_CAT_MAKER_PUZZLE_HASH, &DEFAULT_CAT_MAKER_PUZZLE)
    }

    /// Allocate the dig add incentives action puzzle and return its pointer.
    fn dig_add_incentives_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(DIG_ADD_INCENTIVES_PUZZLE_HASH, &DIG_ADD_INCENTIVES_PUZZLE)
    }

    /// Allocate the dig add mirror action puzzle and return its pointer.
    fn dig_add_mirror_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(DIG_ADD_MIRROR_PUZZLE_HASH, &DIG_ADD_MIRROR_PUZZLE)
    }

    /// Allocate the dig commit incentives action puzzle and return its pointer.
    fn dig_commit_incentives_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            DIG_COMMIT_INCENTIVES_PUZZLE_HASH,
            &DIG_COMMIT_INCENTIVES_PUZZLE,
        )
    }

    /// Allocate the dig initiate payout action puzzle and return its pointer.
    fn dig_initiate_payout_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(DIG_INITIATE_PAYOUT_PUZZLE_HASH, &DIG_INITIATE_PAYOUT_PUZZLE)
    }

    /// Allocate the dig new epoch action puzzle and return its pointer.
    fn dig_new_epoch_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(DIG_NEW_EPOCH_PUZZLE_HASH, &DIG_NEW_EPOCH_PUZZLE)
    }

    /// Allocate the dig remove mirror action puzzle and return its pointer.
    fn dig_remove_mirror_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(DIG_REMOVE_MIRROR_PUZZLE_HASH, &DIG_REMOVE_MIRROR_PUZZLE)
    }

    /// Allocate the dig sync action puzzle and return its pointer.
    fn dig_sync_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(DIG_SYNC_PUZZLE_HASH, &DIG_SYNC_PUZZLE)
    }

    /// Allocate the dig withdraw incentives action puzzle and return its pointer.
    fn dig_withdraw_incentives_action_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            DIG_WITHDRAW_INCENTIVES_PUZZLE_HASH,
            &DIG_WITHDRAW_INCENTIVES_PUZZLE,
        )
    }

    /// Allocate the reserve finalizer puzzle and return its pointer.
    fn reserve_finalizer_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(RESERVE_FINALIZER_PUZZLE_HASH, &RESERVE_FINALIZER_PUZZLE)
    }

    /// Allocate the P2 delegated by singleton puzzle and return its pointer.
    fn p2_delegated_by_singleton_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            P2_DELEGATED_BY_SINGLETON_PUZZLE_HASH,
            &P2_DELEGATED_BY_SINGLETON_PUZZLE,
        )
    }

    /// Allocate the state scheduler puzzle and return its pointer.
    fn state_scheduler_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(STATE_SCHEDULER_PUZZLE_HASH, &STATE_SCHEDULER_PUZZLE)
    }

    /// Allocate the P2 M of N delegate direct puzzle and return its pointer.
    fn p2_m_of_n_delegate_direct_puzzle(&mut self) -> Result<NodePtr, DriverError> {
        self.puzzle(
            P2_M_OF_N_DELEGATE_DIRECT_PUZZLE_HASH,
            &P2_M_OF_N_DELEGATE_DIRECT_PUZZLE,
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
        assert_puzzle_hash!(CATALOG_REFUND_PUZZLE => CATALOG_REFUND_PUZZLE_HASH);
        assert_puzzle_hash!(UNIQUENESS_PRELAUNCHER_PUZZLE => UNIQUENESS_PRELAUNCHER_PUZZLE_HASH);
        assert_puzzle_hash!(PRECOMMIT_LAYER_PUZZLE => PRECOMMIT_LAYER_PUZZLE_HASH);
        assert_puzzle_hash!(SLOT_PUZZLE => SLOT_PUZZLE_HASH);
        assert_puzzle_hash!(ANY_METADATA_UPDATER => ANY_METADATA_UPDATER_HASH);
        assert_puzzle_hash!(VERIFICATION_LAYER_PUZZLE => VERIFICATION_LAYER_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_REGISTER_PUZZLE => XCHANDLES_REGISTER_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_UPDATE_PUZZLE => XCHANDLES_UPDATE_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_EXTEND_PUZZLE => XCHANDLES_EXTEND_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_EXPIRE_PUZZLE => XCHANDLES_EXPIRE_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_ORACLE_PUZZLE => XCHANDLES_ORACLE_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_REFUND_PUZZLE => XCHANDLES_REFUND_PUZZLE_HASH);
        assert_puzzle_hash!(VERIFICATION_PAYMENTS_PUZZLE => VERIFICATION_PAYMENTS_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_FACTOR_PRICING_PUZZLE => XCHANDLES_FACTOR_PRICING_PUZZLE_HASH);
        assert_puzzle_hash!(XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE => XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE_HASH);
        assert_puzzle_hash!(DEFAULT_CAT_MAKER_PUZZLE => DEFAULT_CAT_MAKER_PUZZLE_HASH);
        assert_puzzle_hash!(DIG_ADD_INCENTIVES_PUZZLE => DIG_ADD_INCENTIVES_PUZZLE_HASH);
        assert_puzzle_hash!(DIG_ADD_MIRROR_PUZZLE => DIG_ADD_MIRROR_PUZZLE_HASH);
        assert_puzzle_hash!(DIG_COMMIT_INCENTIVES_PUZZLE => DIG_COMMIT_INCENTIVES_PUZZLE_HASH);
        assert_puzzle_hash!(DIG_INITIATE_PAYOUT_PUZZLE => DIG_INITIATE_PAYOUT_PUZZLE_HASH);
        assert_puzzle_hash!(DIG_NEW_EPOCH_PUZZLE => DIG_NEW_EPOCH_PUZZLE_HASH);
        assert_puzzle_hash!(DIG_REMOVE_MIRROR_PUZZLE => DIG_REMOVE_MIRROR_PUZZLE_HASH);
        assert_puzzle_hash!(DIG_SYNC_PUZZLE => DIG_SYNC_PUZZLE_HASH);
        assert_puzzle_hash!(DIG_WITHDRAW_INCENTIVES_PUZZLE => DIG_WITHDRAW_INCENTIVES_PUZZLE_HASH);
        assert_puzzle_hash!(RESERVE_FINALIZER_PUZZLE => RESERVE_FINALIZER_PUZZLE_HASH);
        assert_puzzle_hash!(P2_DELEGATED_BY_SINGLETON_PUZZLE => P2_DELEGATED_BY_SINGLETON_PUZZLE_HASH);
        assert_puzzle_hash!(STATE_SCHEDULER_PUZZLE => STATE_SCHEDULER_PUZZLE_HASH);
        assert_puzzle_hash!(P2_M_OF_N_DELEGATE_DIRECT_PUZZLE => P2_M_OF_N_DELEGATE_DIRECT_PUZZLE_HASH);
        Ok(())
    }
}
