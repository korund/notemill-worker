//! SHA-256 helpers shared by the model manager and the download module.

use std::path::Path;

use crate::{Error, Result};

pub(crate) enum ShaCheck {
    Ok,
    SkippedNoExpected,
    Mismatch { actual: String, expected: String },
}

pub(crate) fn verify_sha256(path: &Path, expected: Option<&str>) -> Result<ShaCheck> {
    let expected = match expected {
        Some(s) if !s.trim().is_empty() => s.trim().to_ascii_lowercase(),
        _ => return Ok(ShaCheck::SkippedNoExpected),
    };

    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|e| Error::Model(format!("open {} for hashing: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| Error::Model(format!("read {} for hashing: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());

    if actual == expected {
        Ok(ShaCheck::Ok)
    } else {
        Ok(ShaCheck::Mismatch { actual, expected })
    }
}
