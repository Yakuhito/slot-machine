use std::fmt::Debug;

use chia::protocol::Bytes32;
use clvm_traits::{ClvmDecoder, ClvmEncoder, FromClvm, FromClvmError, Raw, ToClvm, ToClvmError};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatNftMetadata {
    pub code: String,
    pub name: String,
    pub description: String,
    pub precision: u8,
    pub image_uris: Vec<String>,
    pub image_hash: Bytes32,
    pub metadata_uris: Vec<String>,
    pub metadata_hash: Bytes32,
    pub license_uris: Vec<String>,
    pub license_hash: Bytes32,
}

impl<N, D: ClvmDecoder<Node = N>> FromClvm<D> for CatNftMetadata {
    fn from_clvm(decoder: &D, node: N) -> Result<Self, FromClvmError> {
        let items: Vec<(String, Raw<N>)> = FromClvm::from_clvm(decoder, node)?;
        let mut metadata = Self::default();

        for (key, Raw(ptr)) in items {
            match key.as_str() {
                "c" => metadata.code = FromClvm::from_clvm(decoder, ptr)?,
                "n" => metadata.name = FromClvm::from_clvm(decoder, ptr)?,
                "d" => metadata.description = FromClvm::from_clvm(decoder, ptr)?,
                "p" => metadata.precision = FromClvm::from_clvm(decoder, ptr)?,
                "u" => metadata.image_uris = FromClvm::from_clvm(decoder, ptr)?,
                "h" => metadata.image_hash = FromClvm::from_clvm(decoder, ptr)?,
                "mu" => metadata.metadata_uris = FromClvm::from_clvm(decoder, ptr)?,
                "mh" => metadata.metadata_hash = FromClvm::from_clvm(decoder, ptr)?,
                "lu" => metadata.license_uris = FromClvm::from_clvm(decoder, ptr)?,
                "lh" => metadata.license_hash = FromClvm::from_clvm(decoder, ptr)?,
                _ => (),
            }
        }

        Ok(metadata)
    }
}

impl<N, E: ClvmEncoder<Node = N>> ToClvm<E> for CatNftMetadata {
    fn to_clvm(&self, encoder: &mut E) -> Result<N, ToClvmError> {
        let items: Vec<(&str, Raw<N>)> = vec![
            ("c", Raw(self.code.to_clvm(encoder)?)),
            ("n", Raw(self.name.to_clvm(encoder)?)),
            ("d", Raw(self.description.to_clvm(encoder)?)),
            ("p", Raw(self.precision.to_clvm(encoder)?)),
            ("u", Raw(self.image_uris.to_clvm(encoder)?)),
            ("h", Raw(self.image_hash.to_clvm(encoder)?)),
            ("mu", Raw(self.metadata_uris.to_clvm(encoder)?)),
            ("mh", Raw(self.metadata_hash.to_clvm(encoder)?)),
            ("lu", Raw(self.license_uris.to_clvm(encoder)?)),
            ("lh", Raw(self.license_hash.to_clvm(encoder)?)),
        ];

        items.to_clvm(encoder)
    }
}
