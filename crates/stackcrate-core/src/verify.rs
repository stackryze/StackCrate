//! Checksum and GPG signature verification for anything downloaded from the
//! network before it is executed or installed (AppImages, archives, .deb
//! files fetched by URL). Shells out to `gpgv`/`gpg` rather than linking a
//! PGP implementation, since every target distro already ships one.

use crate::error::{Error, Result};
use crate::process;
use sha2::{Digest, Sha256, Sha512};
use std::io::Read;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo {
    Sha256,
    Sha512,
}

pub fn hash_file(path: &Path, algo: HashAlgo) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 1 << 16];
    match algo {
        HashAlgo::Sha256 => {
            let mut hasher = Sha256::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hex::encode(hasher.finalize()))
        }
        HashAlgo::Sha512 => {
            let mut hasher = Sha512::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hex::encode(hasher.finalize()))
        }
    }
}

/// Verify `path` matches `expected_hex`. The algorithm is inferred from the
/// length of the expected digest (64 hex chars = sha256, 128 = sha512).
pub fn verify_checksum(path: &Path, expected_hex: &str) -> Result<()> {
    let expected = expected_hex.trim().to_ascii_lowercase();
    let algo = match expected.len() {
        64 => HashAlgo::Sha256,
        128 => HashAlgo::Sha512,
        other => {
            return Err(Error::other(format!(
                "unrecognized checksum length ({other} hex chars); expected 64 (sha256) or 128 (sha512)"
            )));
        }
    };
    let actual = hash_file(path, algo)?;
    if actual == expected {
        Ok(())
    } else {
        Err(Error::ChecksumMismatch {
            path: path.to_path_buf(),
            expected,
            actual,
        })
    }
}

/// Verify a detached GPG signature `sig_path` over `path` using `gpgv` against
/// the user's trusted keyring (falls back to `gpg --verify` if `gpgv` is not
/// present). Requires the signer's key to already be trusted locally; StackCrate
/// never auto-imports keys.
pub fn verify_signature(path: &Path, sig_path: &Path) -> Result<()> {
    if process::binary_available("gpgv") {
        let out = process::run(
            "gpgv",
            &[&path_arg(sig_path), &path_arg(path)],
            Duration::from_secs(20),
        )?;
        return if out.success() {
            Ok(())
        } else {
            Err(Error::SignatureInvalid {
                path: path.to_path_buf(),
                reason: out.combined(),
            })
        };
    }
    if process::binary_available("gpg") {
        let out = process::run(
            "gpg",
            &["--verify", &path_arg(sig_path), &path_arg(path)],
            Duration::from_secs(20),
        )?;
        return if out.success() {
            Ok(())
        } else {
            Err(Error::SignatureInvalid {
                path: path.to_path_buf(),
                reason: out.combined(),
            })
        };
    }
    Err(Error::BinaryMissing {
        binary: "gpgv or gpg".to_string(),
    })
}

fn path_arg(p: &Path) -> String {
    p.to_string_lossy().to_string()
}
