//! Package integrity: a downloaded package must match the digest its release
//! publishes beside it before anything installs it.
//!
//! The digest comes from the same release as the package, so this catches a
//! package altered or truncated between GitHub and disk — not a release that
//! was itself replaced. Pure apart from reading the package file.

use std::fs::File;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::UpdateError;

/// The checksums file `.github/workflows/release.yml` publishes, in
/// `sha256sum` output format.
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS.txt";

/// Length of a hex SHA-256 digest.
const DIGEST_HEX_LEN: usize = 64;

/// Fail unless `package` hashes to the digest `sums` lists for `asset_name`.
pub fn verify_package(package: &Path, asset_name: &str, sums: &str) -> Result<(), UpdateError> {
    let listed = listed_digest(sums, asset_name)
        .ok_or_else(|| UpdateError::MissingChecksum(asset_name.to_owned()))?;
    let actual = file_digest(package).map_err(|e| UpdateError::Io(e.to_string()))?;
    if actual.eq_ignore_ascii_case(listed) {
        Ok(())
    } else {
        Err(UpdateError::ChecksumMismatch(asset_name.to_owned()))
    }
}

/// The digest `sums` lists for `asset_name`. `sha256sum` writes
/// `<hex>  <name>`, or `<hex> *<name>` in binary mode; the name must match
/// exactly, so one package's line never vouches for another.
fn listed_digest<'a>(sums: &'a str, asset_name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (digest, rest) = line.split_once(' ')?;
        let name = rest.strip_prefix([' ', '*'])?;
        let well_formed =
            digest.len() == DIGEST_HEX_LEN && digest.bytes().all(|b| b.is_ascii_hexdigit());
        (well_formed && name == asset_name).then_some(digest)
    })
}

/// Hex SHA-256 of the file at `path`, streamed rather than loaded whole.
fn file_digest(path: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sha256("abc")`.
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn package(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daruda-0.3.0.dmg");
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn a_package_matching_its_listed_digest_passes() {
        let (_dir, path) = package(b"abc");
        let sums = format!("{}  other.zip\n{ABC}  daruda-0.3.0.dmg\n", "0".repeat(64));
        verify_package(&path, "daruda-0.3.0.dmg", &sums).unwrap();
    }

    #[test]
    fn binary_mode_lines_and_uppercase_digests_are_read() {
        let (_dir, path) = package(b"abc");
        let sums = format!("{} *daruda-0.3.0.dmg\n", ABC.to_uppercase());
        verify_package(&path, "daruda-0.3.0.dmg", &sums).unwrap();
    }

    #[test]
    fn an_altered_package_is_refused() {
        let (_dir, path) = package(b"abd");
        let sums = format!("{ABC}  daruda-0.3.0.dmg\n");
        assert!(matches!(
            verify_package(&path, "daruda-0.3.0.dmg", &sums),
            Err(UpdateError::ChecksumMismatch(_))
        ));
    }

    /// Another file's line must not vouch for this one, even as a prefix.
    #[test]
    fn a_package_the_sums_do_not_name_is_refused() {
        let (_dir, path) = package(b"abc");
        let sums = format!("{ABC}  daruda-0.3.0.dmg.old\n{ABC}  x-daruda-0.3.0.dmg\n");
        assert!(matches!(
            verify_package(&path, "daruda-0.3.0.dmg", &sums),
            Err(UpdateError::MissingChecksum(_))
        ));
    }

    #[test]
    fn a_malformed_digest_is_not_a_listing() {
        assert_eq!(listed_digest("abc  daruda.dmg", "daruda.dmg"), None);
        assert_eq!(
            listed_digest(&format!("{ABC}daruda.dmg"), "daruda.dmg"),
            None
        );
    }
}
