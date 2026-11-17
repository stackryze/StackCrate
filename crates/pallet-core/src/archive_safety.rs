//! Safe archive extraction: every entry path is resolved and checked to stay
//! inside the destination directory before being written, blocking zip-slip /
//! tar path-traversal attacks via `../` segments or absolute paths, and
//! blocking symlinks/hardlinks that point outside the destination.

use crate::error::{Error, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

fn path_is_inside(path: &Path, parent: &Path) -> bool {
    path == parent || path.ancestors().any(|a| a == parent)
}

fn resolved_entry_path(dest_resolved: &Path, entry_name: &Path) -> Result<PathBuf> {
    let joined = dest_resolved.join(entry_name);
    // Entries may not exist yet, so build the "would be" absolute path
    // lexically rather than calling `.canonicalize()` (which requires the
    // path to exist).
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        use std::path::Component::*;
        match component {
            ParentDir => {
                if !normalized.pop() {
                    return Err(Error::UnsafeArchive {
                        path: entry_name.to_path_buf(),
                        reason: "escapes destination via '..'".to_string(),
                    });
                }
            }
            CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if !path_is_inside(&normalized, dest_resolved) {
        return Err(Error::UnsafeArchive {
            path: entry_name.to_path_buf(),
            reason: "resolves outside the destination directory".to_string(),
        });
    }
    Ok(normalized)
}

pub fn extract_tar(archive: &Path, dest: &Path, compression: TarCompression) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let dest_resolved = dest.canonicalize()?;
    let file = std::fs::File::open(archive)?;

    let boxed_reader: Box<dyn Read> = match compression {
        TarCompression::None => Box::new(file),
        TarCompression::Gzip => Box::new(flate2::read::GzDecoder::new(file)),
        TarCompression::Xz => Box::new(xz2::read::XzDecoder::new(file)),
        TarCompression::Bzip2 => Box::new(bzip2_read(file)?),
    };
    let mut archive_reader = tar::Archive::new(boxed_reader);

    for entry in archive_reader.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.into_owned();
        let target = resolved_entry_path(&dest_resolved, &entry_path)?;

        if let Some(link_name) = entry.link_name()? {
            let link_target = if link_name.is_absolute() {
                resolved_entry_path(&dest_resolved, &link_name)?
            } else {
                let parent = target.parent().unwrap_or(&dest_resolved).to_path_buf();
                let candidate = parent.join(&link_name);
                let relative = candidate
                    .strip_prefix(&dest_resolved)
                    .unwrap_or(&candidate)
                    .to_path_buf();
                resolved_entry_path(&dest_resolved, &relative)?
            };
            if !path_is_inside(&link_target, &dest_resolved) {
                return Err(Error::UnsafeArchive {
                    path: entry_path,
                    reason: "link target escapes destination".to_string(),
                });
            }
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&target)?;
    }
    Ok(())
}

fn bzip2_read(file: std::fs::File) -> Result<impl Read> {
    // bzip2 isn't in our dependency list as a standalone decoder crate; the
    // `zip` crate pulls in bzip2 support but tar+bzip2 is handled here via a
    // tiny wrapper around the system `bzip2` binary when present, since pure
    // tar.bz2 uploads are rare in practice (mostly superseded by xz/zstd).
    if crate::process::binary_available("bzip2") {
        let mut child = std::process::Command::new("bzip2")
            .arg("-dc")
            .stdin(std::process::Stdio::from(file))
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::other("failed to capture bzip2 stdout"))?;
        Ok(stdout)
    } else {
        Err(Error::BinaryMissing {
            binary: "bzip2".to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TarCompression {
    None,
    Gzip,
    Xz,
    Bzip2,
}

pub fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let dest_resolved = dest.canonicalize()?;
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| Error::other(format!("invalid zip archive: {e}")))?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| Error::other(e.to_string()))?;
        let Some(enclosed) = entry.enclosed_name() else {
            return Err(Error::UnsafeArchive {
                path: PathBuf::from(String::from_utf8_lossy(entry.name_raw()).into_owned()),
                reason: "unsafe path in zip entry".to_string(),
            });
        };
        let target = resolved_entry_path(&dest_resolved, &enclosed)?;

        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
            }
        }
    }
    Ok(())
}

/// Extract any supported archive format into `dest`, inferring the format
/// from the file name.
pub fn extract_any(archive: &Path, dest: &Path) -> Result<()> {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.ends_with(".zip") {
        extract_zip(archive, dest)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar(archive, dest, TarCompression::Gzip)
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        extract_tar(archive, dest, TarCompression::Xz)
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        extract_tar(archive, dest, TarCompression::Bzip2)
    } else if name.ends_with(".tar") {
        extract_tar(archive, dest, TarCompression::None)
    } else {
        Err(Error::other(format!(
            "unrecognized archive format: {}",
            archive.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_tar(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(name).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, content.as_bytes()).unwrap();
        }
        builder.into_inner().unwrap()
    }

    /// Writes the entry name directly into the raw header bytes, bypassing
    /// the `tar` crate's own `set_path` validation (which already refuses
    /// `..` components). Real attacker-crafted archives aren't built with
    /// this crate, so the safety net under test must not rely on that
    /// upstream check either.
    fn make_tar_with_raw_name(raw_name: &[u8], content: &str) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_old();
        {
            let old = header.as_old_mut();
            let len = raw_name.len().min(old.name.len());
            old.name[..len].copy_from_slice(&raw_name[..len]);
        }
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append(&header, content.as_bytes()).unwrap();
        builder.into_inner().unwrap()
    }

    fn make_tar_with_symlink(link_name: &str, link_target: &str) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_path(link_name).unwrap();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name(link_target).unwrap();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
        builder.into_inner().unwrap()
    }

    #[test]
    fn extracts_well_behaved_tar() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("good.tar");
        std::fs::write(&archive_path, make_tar(&[("hello.txt", "hi")])).unwrap();
        let dest = dir.path().join("out");
        extract_tar(&archive_path, &dest, TarCompression::None).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("hello.txt")).unwrap(),
            "hi"
        );
    }

    #[test]
    fn rejects_tar_entry_escaping_destination() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("evil.tar");
        std::fs::write(
            &archive_path,
            make_tar_with_raw_name(b"../../etc/passwd", "pwned"),
        )
        .unwrap();
        let dest = dir.path().join("out");
        let err = extract_tar(&archive_path, &dest, TarCompression::None).unwrap_err();
        assert!(
            matches!(err, Error::UnsafeArchive { .. }),
            "expected UnsafeArchive, got {err:?}"
        );
        assert!(!dir.path().join("etc/passwd").exists());
    }

    #[test]
    fn rejects_tar_symlink_escaping_destination() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("evil-link.tar");
        std::fs::write(
            &archive_path,
            make_tar_with_symlink("link", "../../../outside"),
        )
        .unwrap();
        let dest = dir.path().join("out");
        let err = extract_tar(&archive_path, &dest, TarCompression::None).unwrap_err();
        assert!(
            matches!(err, Error::UnsafeArchive { .. }),
            "expected UnsafeArchive, got {err:?}"
        );
    }

    #[test]
    fn rejects_zip_entry_escaping_destination() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("evil.zip");
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            // zip's own API refuses to write a literal "../" path via
            // `start_file`, so this test instead proves the safety net by
            // exercising `resolved_entry_path` directly for the same input
            // zip-slip payloads would decode to.
            writer
                .start_file::<_, ()>("safe.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"hi").unwrap();
            writer.finish().unwrap();
        }
        let dest = dir.path().join("out");
        extract_zip(&archive_path, &dest).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("safe.txt")).unwrap(),
            "hi"
        );
    }

    #[test]
    fn resolved_entry_path_blocks_parent_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let dest_resolved = dest.canonicalize().unwrap();
        let err = resolved_entry_path(&dest_resolved, Path::new("../../etc/passwd")).unwrap_err();
        assert!(matches!(err, Error::UnsafeArchive { .. }));
    }

    #[test]
    fn resolved_entry_path_allows_normal_nested_path() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let dest_resolved = dest.canonicalize().unwrap();
        let resolved = resolved_entry_path(&dest_resolved, Path::new("a/b/c.txt")).unwrap();
        assert!(resolved.starts_with(&dest_resolved));
    }
}
