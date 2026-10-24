//! Signature checks for apps the user has given a trusted key.
//!
//! A checksum published next to an asset only shows the download is intact.
//! A signature made with a key the user trusts shows the asset comes from
//! whoever holds that key, even if the release itself was tampered with.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use minisign_verify::{PublicKey, Signature as Minisig};

use crate::error::{Error, Result};
use crate::model::{Asset, Release, Signature};
use crate::source::Source;
use crate::{host, http, store, verify};

/// What will be checked for an asset.
#[derive(Debug, Clone)]
pub enum Signed {
    /// A detached signature of the asset itself.
    Asset { sig: Asset },
    /// A signed checksum list that names the asset.
    Checksums { list: Asset, sig: Asset },
    /// A GitHub artifact attestation, checked with `gh`.
    Attestation,
}

impl Signed {
    /// Short description for confirmation prompts.
    pub fn describe(&self) -> String {
        match self {
            Signed::Asset { sig } => sig.name.clone(),
            Signed::Checksums { list, sig } => format!("{} signing {}", sig.name, list.name),
            Signed::Attestation => "GitHub attestation".into(),
        }
    }
}

/// Finds the signature to check for `asset`. Fails if the release has none,
/// since a trusted key means unsigned assets must not be installed.
pub fn find(signature: &Signature, asset: &Asset, release: &Release) -> Result<Signed> {
    let suffixes: &[&str] = match signature {
        Signature::Minisign { .. } => &[".minisig"],
        Signature::Gpg { .. } => &[".asc", ".sig", ".gpg"],
        Signature::Attestation => return Ok(Signed::Attestation),
    };
    let sig_for = |name: &str| {
        release.assets.iter().find(|a| {
            suffixes
                .iter()
                .any(|suffix| a.name.eq_ignore_ascii_case(&format!("{name}{suffix}")))
        })
    };

    if let Some(sig) = sig_for(&asset.name) {
        return Ok(Signed::Asset { sig: sig.clone() });
    }
    release
        .assets
        .iter()
        .filter(|a| verify::is_checksum_list(&a.name.to_ascii_lowercase()))
        .find_map(|list| {
            sig_for(&list.name).map(|sig| Signed::Checksums {
                list: list.clone(),
                sig: sig.clone(),
            })
        })
        .ok_or_else(|| Error::SignatureMissing(asset.name.clone()))
}

/// Checks the downloaded `file` (whose SHA-256 is `sha256`).
pub fn check(signature: &Signature, signed: &Signed, source: &Source, file: &Path, sha256: &str) -> Result<()> {
    match signed {
        Signed::Attestation => attestation(source, file),
        Signed::Asset { sig } => {
            let sig = http::get_bytes(&sig.url)?;
            check_file(signature, &sig, file)
        }
        Signed::Checksums { list, sig } => {
            let list_bytes = http::get_bytes(&list.url)?;
            let sig = http::get_bytes(&sig.url)?;
            let tmp = scratch_file()?;
            fs::write(tmp.path(), &list_bytes)?;
            check_file(signature, &sig, tmp.path())?;
            let name = file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
            match verify::find_in_checksums(&String::from_utf8_lossy(&list_bytes), &name, false) {
                Some(hex) if hex == sha256 => Ok(()),
                Some(hex) => Err(Error::ChecksumMismatch {
                    asset: name.into_owned(),
                    expected: hex,
                    actual: sha256.to_string(),
                }),
                None => Err(Error::SignatureInvalid(format!("{} does not list {name}", list.name))),
            }
        }
    }
}

fn check_file(signature: &Signature, sig: &[u8], file: &Path) -> Result<()> {
    match signature {
        Signature::Minisign { public_key } => minisign(public_key, sig, file),
        Signature::Gpg { keyring } => gpg(keyring, sig, file),
        Signature::Attestation => unreachable!("attestations have no signature file"),
    }
}

fn minisign(public_key: &str, sig: &[u8], file: &Path) -> Result<()> {
    let invalid = |e: minisign_verify::Error| Error::SignatureInvalid(format!("minisign: {e}"));
    let key = PublicKey::from_base64(public_key).map_err(invalid)?;
    let sig = Minisig::decode(&String::from_utf8_lossy(sig)).map_err(invalid)?;
    match key.verify_stream(&sig) {
        Ok(mut verifier) => {
            let mut reader = File::open(file)?;
            let mut buf = vec![0; 64 * 1024];
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                verifier.update(&buf[..n]);
            }
            verifier.finalize().map_err(invalid)
        }
        // Signatures from minisign before 0.8 hash nothing up front and need
        // the whole file in memory.
        Err(minisign_verify::Error::UnsupportedLegacyMode) => key.verify(&fs::read(file)?, &sig, true).map_err(invalid),
        Err(e) => Err(invalid(e)),
    }
}

fn gpg(keyring: &Path, sig: &[u8], file: &Path) -> Result<()> {
    let sig_file = scratch_file()?;
    fs::write(sig_file.path(), sig)?;
    let out = host::command("gpgv")
        .arg("--keyring")
        .arg(keyring)
        .arg(sig_file.path())
        .arg(file)
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::SignatureInvalid(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

fn attestation(source: &Source, file: &Path) -> Result<()> {
    let Source::GitHub { owner, repo } = source else {
        return Err(Error::SignatureInvalid(
            "attestations are only available on GitHub".into(),
        ));
    };
    let out = host::command("gh")
        .args(["attestation", "verify"])
        .arg(file)
        .args(["--repo", &format!("{owner}/{repo}")])
        .output()
        .map_err(|e| Error::SignatureInvalid(format!("cannot run gh: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::SignatureInvalid(format!(
            "gh attestation verify: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// A signature requirement as the user gives it, before keys are stored.
#[derive(Debug, Clone)]
pub enum SignatureInput {
    /// A base64 minisign key, or a path to a `minisign.pub` file.
    Minisign(String),
    /// An OpenPGP public key file, armored or binary.
    GpgKeyFile(PathBuf),
    Attestation,
}

/// Validates `input` and stores what it needs (a GPG keyring for app `id`).
pub fn resolve(input: &SignatureInput, id: &str, source: &Source) -> Result<Signature> {
    match input {
        SignatureInput::Minisign(key) => Ok(Signature::Minisign {
            public_key: parse_minisign_key(key)?,
        }),
        SignatureInput::GpgKeyFile(file) => Ok(Signature::Gpg {
            keyring: store_gpg_key(id, file)?,
        }),
        SignatureInput::Attestation if matches!(source, Source::GitHub { .. }) => Ok(Signature::Attestation),
        SignatureInput::Attestation => Err(Error::SignatureInvalid(
            "attestations are only available for GitHub projects".into(),
        )),
    }
}

/// Reads a minisign public key given either as the base64 key itself or as
/// a path to a `minisign.pub` file, and returns the base64 key.
pub fn parse_minisign_key(key_or_path: &str) -> Result<String> {
    let invalid = |e: minisign_verify::Error| Error::SignatureInvalid(format!("minisign key: {e}"));
    let text = match fs::read_to_string(key_or_path) {
        Ok(text) => text,
        Err(_) => key_or_path.to_string(),
    };
    let key = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .unwrap_or_default()
        .to_string();
    PublicKey::from_base64(&key).map_err(invalid)?;
    Ok(key)
}

/// Stores an OpenPGP public key (armored or binary) as a keyring that `gpgv`
/// reads, at `$XDG_DATA_HOME/forager/keys/<id>.gpg`. Nothing is imported
/// into the user's own GnuPG keyring.
pub fn store_gpg_key(id: &str, key_file: &Path) -> Result<PathBuf> {
    let dir = dirs::data_dir().ok_or(Error::NoHome)?.join("forager").join("keys");
    store_gpg_key_in(&dir, id, key_file)
}

fn store_gpg_key_in(dir: &Path, id: &str, key_file: &Path) -> Result<PathBuf> {
    let key = fs::read(key_file)?;
    let binary = if key.starts_with(b"-----BEGIN PGP") {
        let out = host::command("gpg")
            .arg("--dearmor")
            .arg("--output")
            .arg("-")
            .arg(key_file)
            .output()?;
        if !out.status.success() {
            return Err(Error::SignatureInvalid(format!(
                "gpg --dearmor: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        out.stdout
    } else {
        key
    };
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{id}.gpg"));
    fs::write(&path, binary)?;
    Ok(path)
}

/// A temporary file under Forager's cache rather than `/tmp`: inside a
/// Flatpak, programs run on the host, which cannot see the sandbox's `/tmp`.
fn scratch_file() -> Result<tempfile::NamedTempFile> {
    let dir = store::cache_dir();
    fs::create_dir_all(&dir)?;
    Ok(tempfile::NamedTempFile::new_in(dir)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.into(),
            url: String::new(),
            size: 0,
            digest: None,
        }
    }

    fn release(names: &[&str]) -> Release {
        Release {
            tag: "v1".into(),
            name: None,
            prerelease: false,
            published_at: None,
            assets: names.iter().map(|n| asset(n)).collect(),
        }
    }

    const KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";

    #[test]
    fn prefers_asset_signature_then_signed_checksums() {
        let gpg = Signature::Gpg {
            keyring: "/k.gpg".into(),
        };
        let release = release(&["tool.tar.gz", "tool.tar.gz.asc", "SHA256SUMS", "SHA256SUMS.asc"]);
        assert!(matches!(
            find(&gpg, &asset("tool.tar.gz"), &release).unwrap(),
            Signed::Asset { sig } if sig.name == "tool.tar.gz.asc"
        ));

        let release = self::release(&["tool.tar.gz", "SHA256SUMS", "SHA256SUMS.sig"]);
        assert!(matches!(
            find(&gpg, &asset("tool.tar.gz"), &release).unwrap(),
            Signed::Checksums { list, .. } if list.name == "SHA256SUMS"
        ));

        let minisign = Signature::Minisign { public_key: KEY.into() };
        assert!(matches!(
            find(&minisign, &asset("tool.tar.gz"), &release),
            Err(Error::SignatureMissing(_))
        ));
    }

    #[test]
    fn parses_minisign_key_or_pub_file() {
        assert_eq!(parse_minisign_key(KEY).unwrap(), KEY);
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("minisign.pub");
        fs::write(&file, format!("untrusted comment: minisign public key\n{KEY}\n")).unwrap();
        assert_eq!(parse_minisign_key(file.to_str().unwrap()).unwrap(), KEY);
        assert!(parse_minisign_key("not a key").is_err());
    }

    #[test]
    fn rejects_bad_minisign_signature() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("tool");
        fs::write(&file, "data").unwrap();
        assert!(matches!(
            minisign(KEY, b"untrusted comment: x\nnot a signature\n", &file),
            Err(Error::SignatureInvalid(_))
        ));
    }
}
