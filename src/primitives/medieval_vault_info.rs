use chia::{
    bls::PublicKey, clvm_utils::TreeHash, protocol::Bytes32, puzzles::singleton::SingletonArgs,
};
use chia_wallet_sdk::SingletonLayer;

use crate::{MOfNLayer, P2MOfNDelegateDirectArgs};

type MedievalVaultLayers = SingletonLayer<MOfNLayer>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MedievalVaultInfo {
    pub launcher_id: Bytes32,

    pub m: usize,
    pub public_key_list: Vec<PublicKey>,
}

impl MedievalVaultInfo {
    pub fn new(launcher_id: Bytes32, m: usize, public_key_list: Vec<PublicKey>) -> Self {
        Self {
            launcher_id,
            m,
            public_key_list,
        }
    }

    pub fn inner_puzzle_hash(&self) -> TreeHash {
        SingletonArgs::curry_tree_hash(
            self.launcher_id,
            P2MOfNDelegateDirectArgs::curry_tree_hash(self.m, self.public_key_list.clone()),
        )
    }

    pub fn into_layers(&self) -> MedievalVaultLayers {
        SingletonLayer::new(
            self.launcher_id,
            MOfNLayer::new(self.m, self.public_key_list.clone()),
        )
    }
}
