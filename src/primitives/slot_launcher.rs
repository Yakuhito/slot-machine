use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::singleton::SINGLETON_LAUNCHER_PUZZLE_HASH,
};
use chia_wallet_sdk::{DriverError, Launcher, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::SpendContextExt;

use super::slot;

/// Used to create slots & then transition to either a new
/// slot launcher or the main logic singleton innerpuzzle
#[derive(Debug, Clone)]
#[must_use]
pub struct SlotLauncher {
    pub coin: Coin,
    pub launcher_id: Bytes32,
    pub slot_value_hashes: Vec<Bytes32>,
    pub next_puzzle_hash: Bytes32,
}

impl SlotLauncher {
    pub fn from_coin(
        coin: Coin,
        launcher_id: Bytes32,
        slot_value_hashes: Vec<Bytes32>,
        next_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            coin,
            launcher_id,
            slot_value_hashes,
            next_puzzle_hash,
        }
    }

    // TODO from here
    // pub fn new(
    //     ctx: &mut SpendContext,
    //     slot_value_hashes: Vec<Bytes32>,
    //     next_puzzle_hash: Bytes32,
    // ) -> Self {
    //     Self::from_coin(Coin::new(), slot_value_hashes, next_puzzle_hash)
    // }
}
