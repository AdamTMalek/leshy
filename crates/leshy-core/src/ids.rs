use std::fmt::{Display, Formatter};

use crate::path::RelativePath;

/// FNV-1a 64-bit offset basis used as the deterministic hash seed for stable IDs.
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
/// FNV-1a 64-bit prime used for each byte-mixing step in stable ID hashing.
const FNV_PRIME: u64 = 0x100000001b3;

/// Defines a strongly typed ID newtype with a stable text prefix, `Display` formatting,
/// and helpers used by deterministic graph hashing.
macro_rules! typed_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub fn as_u64(self) -> u64 {
                self.0
            }

            pub(crate) fn stable_component(self) -> String {
                format!(concat!($label, ":{:016x}"), self.0)
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!($label, ":{:016x}"), self.0)
            }
        }
    };
}

typed_id!(RepositoryId, "repo");
typed_id!(DirectoryId, "dir");
typed_id!(FileId, "file");
typed_id!(SymbolId, "symbol");

impl RepositoryId {
    /// Builds a deterministic repository identifier from a stable key.
    pub fn new(stable_key: &str) -> Self {
        Self(stable_hash(&["repository", stable_key]))
    }
}

impl DirectoryId {
    /// Builds a deterministic directory identifier from the repository and relative path.
    pub fn new(repository_id: RepositoryId, relative_path: &RelativePath) -> Self {
        Self(stable_hash(&[
            "directory",
            &repository_id.stable_component(),
            relative_path.as_str(),
        ]))
    }
}

impl FileId {
    /// Builds a deterministic file identifier from the repository and relative path.
    pub fn new(repository_id: RepositoryId, relative_path: &RelativePath) -> Self {
        Self(stable_hash(&[
            "file",
            &repository_id.stable_component(),
            relative_path.as_str(),
        ]))
    }
}

impl SymbolId {
    /// Builds a deterministic symbol identifier from its defining file and stable key.
    pub fn new(file_id: FileId, stable_key: &str) -> Self {
        Self(stable_hash(&[
            "symbol",
            &file_id.stable_component(),
            stable_key,
        ]))
    }
}

pub(crate) fn stable_hash(parts: &[&str]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;

    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            hash = update_fnv(hash, &[0x1f]);
        }
        hash = update_fnv(hash, part.as_bytes());
    }

    hash
}

fn update_fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    hash
}
