mod launch;
mod merkle;
mod sync;
mod update;
mod view;

use chia_protocol::Bytes32;
use chia_wallet_sdk::driver::DelegatedPuzzle;

pub use launch::*;
pub use merkle::*;
pub use sync::*;
pub use update::*;
pub use view::*;

pub fn oracle_delegated_puzzles() -> Vec<DelegatedPuzzle> {
    vec![DelegatedPuzzle::Oracle(Bytes32::default(), 0)]
}
