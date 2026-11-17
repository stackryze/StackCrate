//! Detect what kind of installable thing a file is, by extension first and
//! magic bytes as a fallback (useful for downloaded files with generic names).

use crate::types::SourceId;
use std::io::Read;
use std::path::Path;

pub fn detect_by_extension(path: &Path) -> Option<SourceId> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".deb") {
        return Some(SourceId::DebFile);
    }
    if name.ends_with(".appimage") {
        return Some(SourceId::AppImage);
    }
    const ARCHIVE_SUFFIXES: &[&str] = &[
        ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tar", ".zip",
    ];
    if ARCHIVE_SUFFIXES.iter().any(|s| name.ends_with(s)) {
        return Some(SourceId::Archive);
    }
    None
}

pub fn detect_by_magic(path: &Path) -> std::io::Result<Option<SourceId>> {
    let mut file = std::fs::File::open(path)?;
    let mut head = [0u8; 16];
    let n = file.read(&mut head)?;
    let head = &head[..n];

    if head.starts_with(b"!<arch>\n") {
        return Ok(Some(SourceId::DebFile));
    }
    let is_elf = head.starts_with(b"\x7fELF");
    let is_appimage_marker =
        head.len() >= 11 && (&head[8..11] == b"AI\x01" || &head[8..11] == b"AI\x02");
    if is_elf && is_appimage_marker {
        return Ok(Some(SourceId::AppImage));
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Ok(Some(SourceId::Archive));
    }
    if head.starts_with(b"\x1f\x8b") // gzip
        || head.starts_with(b"BZh") // bzip2
        || head.starts_with(b"\xfd7zXZ\x00")
    // xz
    {
        return Ok(Some(SourceId::Archive));
    }
    Ok(None)
}

pub fn detect(path: &Path) -> std::io::Result<Option<SourceId>> {
    if let Some(kind) = detect_by_extension(path) {
        return Ok(Some(kind));
    }
    detect_by_magic(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_extensions() {
        assert_eq!(
            detect_by_extension(Path::new("app.deb")),
            Some(SourceId::DebFile)
        );
        assert_eq!(
            detect_by_extension(Path::new("App-x86_64.AppImage")),
            Some(SourceId::AppImage)
        );
        assert_eq!(
            detect_by_extension(Path::new("thing.tar.gz")),
            Some(SourceId::Archive)
        );
        assert_eq!(
            detect_by_extension(Path::new("thing.tgz")),
            Some(SourceId::Archive)
        );
        assert_eq!(
            detect_by_extension(Path::new("thing.tar.xz")),
            Some(SourceId::Archive)
        );
        assert_eq!(
            detect_by_extension(Path::new("thing.zip")),
            Some(SourceId::Archive)
        );
        assert_eq!(detect_by_extension(Path::new("no-extension-here")), None);
    }

    #[test]
    fn detects_deb_by_ar_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mystery-file");
        std::fs::write(&path, b"!<arch>\ndebian-binary...").unwrap();
        assert_eq!(detect_by_magic(&path).unwrap(), Some(SourceId::DebFile));
    }

    #[test]
    fn detects_zip_by_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mystery-file");
        std::fs::write(&path, [0x50, 0x4b, 0x03, 0x04, 0, 0]).unwrap();
        assert_eq!(detect_by_magic(&path).unwrap(), Some(SourceId::Archive));
    }

    #[test]
    fn plain_elf_without_appimage_marker_is_unrecognized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("some-binary");
        let mut bytes = vec![0x7f, b'E', b'L', b'F'];
        bytes.extend(std::iter::repeat_n(0u8, 16));
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(detect_by_magic(&path).unwrap(), None);
    }
}
