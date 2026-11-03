use std::io::{Read, Write};

use crate::Bundle;

/// Bumped whenever the serialized layout changes; older files fail to load.
pub const BUNDLE_FORMAT_VERSION: u32 = 1;

const MAGIC: &[u8; 8] = b"GLINTCRP";

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("not a Glint corpus bundle")]
    BadMagic,
    #[error("bundle format version {found} is not supported (expected {BUNDLE_FORMAT_VERSION})")]
    UnsupportedVersion { found: u32 },
    #[error("bundle is damaged: {0}")]
    Damaged(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Serializes `bundle` as magic, format version, then zstd-compressed JSON.
pub fn write_bundle(bundle: &Bundle, out: &mut impl Write) -> Result<(), BundleError> {
    out.write_all(MAGIC)?;
    out.write_all(&BUNDLE_FORMAT_VERSION.to_le_bytes())?;
    let json = serde_json::to_vec(bundle).map_err(|e| BundleError::Damaged(e.to_string()))?;
    let compressed = zstd::encode_all(json.as_slice(), 19)?;
    out.write_all(&compressed)?;
    Ok(())
}

pub fn read_bundle(input: &mut impl Read) -> Result<Bundle, BundleError> {
    let mut magic = [0_u8; 8];
    input
        .read_exact(&mut magic)
        .map_err(|_| BundleError::BadMagic)?;
    if &magic != MAGIC {
        return Err(BundleError::BadMagic);
    }
    let mut version = [0_u8; 4];
    input
        .read_exact(&mut version)
        .map_err(|_| BundleError::BadMagic)?;
    let found = u32::from_le_bytes(version);
    if found != BUNDLE_FORMAT_VERSION {
        return Err(BundleError::UnsupportedVersion { found });
    }
    let mut compressed = Vec::new();
    input.read_to_end(&mut compressed)?;
    let json = zstd::decode_all(compressed.as_slice())?;
    serde_json::from_slice(&json).map_err(|e| BundleError::Damaged(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut bundle = Bundle::default();
        bundle.info.version = "2026-11-03".to_owned();
        let mut bytes = Vec::new();
        write_bundle(&bundle, &mut bytes).unwrap();
        let back = read_bundle(&mut bytes.as_slice()).unwrap();
        assert_eq!(back, bundle);
    }

    #[test]
    fn wrong_version_fails_loudly() {
        let mut bytes = Vec::new();
        write_bundle(&Bundle::default(), &mut bytes).unwrap();
        bytes[8] = 99;
        assert!(matches!(
            read_bundle(&mut bytes.as_slice()),
            Err(BundleError::UnsupportedVersion { found: 99 })
        ));
    }
}
