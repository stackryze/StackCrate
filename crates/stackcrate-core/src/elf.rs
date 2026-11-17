//! Minimal ELF64 section reader: just enough to pull one named section's
//! raw bytes out of a file (used to read an AppImage's embedded
//! `.upd_info` update-source string). Hand-rolled rather than pulling in a
//! parsing crate: the format needed is a handful of fixed-offset reads from
//! the section header table, well within what's comfortable to get right
//! and unit-test directly against a synthetic buffer, for one narrow need.
//! Only 64-bit little-endian ELF is supported (the overwhelming majority of
//! real AppImages); anything else returns `None` rather than guessing.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1; // little-endian

/// Reads the raw bytes of the named ELF section from the file at `path`,
/// or `None` if it isn't a (supported) ELF file or has no such section.
pub fn read_section(path: &Path, section_name: &str) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header).ok()?;
    read_section_from(&mut file, &header, section_name)
}

fn read_section_from(
    file: &mut (impl Read + Seek),
    header: &[u8; 64],
    section_name: &str,
) -> Option<Vec<u8>> {
    if header[0..4] != ELF_MAGIC || header[4] != ELFCLASS64 || header[5] != ELFDATA2LSB {
        return None;
    }
    let shoff = u64::from_le_bytes(header[40..48].try_into().ok()?);
    let shentsize = u16::from_le_bytes(header[58..60].try_into().ok()?) as u64;
    let shnum = u16::from_le_bytes(header[60..62].try_into().ok()?) as u64;
    let shstrndx = u16::from_le_bytes(header[62..64].try_into().ok()?) as u64;
    if shnum == 0 || shentsize < 64 {
        return None;
    }

    let strtab_hdr = read_shdr(file, shoff, shentsize, shstrndx)?;
    let strtab_offset = u64::from_le_bytes(strtab_hdr[24..32].try_into().ok()?);
    let strtab_size = u64::from_le_bytes(strtab_hdr[32..40].try_into().ok()?);
    file.seek(SeekFrom::Start(strtab_offset)).ok()?;
    let mut strtab = vec![0u8; strtab_size as usize];
    file.read_exact(&mut strtab).ok()?;

    for index in 0..shnum {
        let shdr = read_shdr(file, shoff, shentsize, index)?;
        let name_offset = u32::from_le_bytes(shdr[0..4].try_into().ok()?) as usize;
        if read_c_str(&strtab, name_offset) != Some(section_name) {
            continue;
        }
        let offset = u64::from_le_bytes(shdr[24..32].try_into().ok()?);
        let size = u64::from_le_bytes(shdr[32..40].try_into().ok()?);
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut data = vec![0u8; size as usize];
        file.read_exact(&mut data).ok()?;
        return Some(data);
    }
    None
}

fn read_shdr(
    file: &mut (impl Read + Seek),
    shoff: u64,
    shentsize: u64,
    index: u64,
) -> Option<[u8; 64]> {
    file.seek(SeekFrom::Start(shoff + index * shentsize)).ok()?;
    let mut buf = [0u8; 64];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn read_c_str(buf: &[u8], offset: usize) -> Option<&str> {
    let bytes = buf.get(offset..)?;
    let end = bytes.iter().position(|&b| b == 0)?;
    std::str::from_utf8(&bytes[..end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Builds a minimal, otherwise-empty ELF64 file with exactly one named
    /// section containing `content`, laid out as:
    /// [64-byte ELF header][section data][shstrtab][section header table]
    fn synthetic_elf(section_name: &str, content: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 64]; // ELF header, patched below
        buf[0..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = ELFCLASS64;
        buf[5] = ELFDATA2LSB;

        let section_offset = buf.len() as u64;
        buf.extend_from_slice(content);

        // shstrtab: a leading NUL (so index 0 means "no name"), then the
        // section's own name, NUL-terminated.
        let strtab_offset = buf.len() as u64;
        let mut strtab = vec![0u8];
        let name_offset_in_strtab = strtab.len() as u32;
        strtab.extend_from_slice(section_name.as_bytes());
        strtab.push(0);
        let shstrtab_name_offset = strtab.len() as u32; // ".shstrtab" itself, unused by the reader
        strtab.extend_from_slice(b".shstrtab\0");
        let strtab_size = strtab.len() as u64;
        buf.extend_from_slice(&strtab);

        let shoff = buf.len() as u64;
        // Section 0: the null section (required by spec; index 0 is never matched).
        buf.extend_from_slice(&[0u8; 64]);
        // Section 1: our named section.
        let mut shdr = vec![0u8; 64];
        shdr[0..4].copy_from_slice(&name_offset_in_strtab.to_le_bytes());
        shdr[24..32].copy_from_slice(&section_offset.to_le_bytes());
        shdr[32..40].copy_from_slice(&(content.len() as u64).to_le_bytes());
        buf.extend_from_slice(&shdr);
        // Section 2: the shstrtab itself.
        let mut shstrtab_hdr = vec![0u8; 64];
        shstrtab_hdr[0..4].copy_from_slice(&shstrtab_name_offset.to_le_bytes());
        shstrtab_hdr[24..32].copy_from_slice(&strtab_offset.to_le_bytes());
        shstrtab_hdr[32..40].copy_from_slice(&strtab_size.to_le_bytes());
        buf.extend_from_slice(&shstrtab_hdr);

        buf[40..48].copy_from_slice(&shoff.to_le_bytes());
        buf[58..60].copy_from_slice(&64u16.to_le_bytes()); // e_shentsize
        buf[60..62].copy_from_slice(&3u16.to_le_bytes()); // e_shnum
        buf[62..64].copy_from_slice(&2u16.to_le_bytes()); // e_shstrndx

        buf
    }

    #[test]
    fn finds_an_existing_section() {
        let bytes = synthetic_elf(
            ".upd_info",
            b"gh-releases-zsync|owner|repo|latest|*.AppImage",
        );
        let mut cursor = Cursor::new(bytes.clone());
        let header: [u8; 64] = bytes[0..64].try_into().unwrap();
        let found = read_section_from(&mut cursor, &header, ".upd_info").unwrap();
        assert_eq!(found, b"gh-releases-zsync|owner|repo|latest|*.AppImage");
    }

    #[test]
    fn returns_none_for_missing_section() {
        let bytes = synthetic_elf(".upd_info", b"whatever");
        let mut cursor = Cursor::new(bytes.clone());
        let header: [u8; 64] = bytes[0..64].try_into().unwrap();
        assert!(read_section_from(&mut cursor, &header, ".does_not_exist").is_none());
    }

    #[test]
    fn rejects_non_elf_input() {
        let header = [0u8; 64];
        let mut cursor = Cursor::new(vec![0u8; 128]);
        assert!(read_section_from(&mut cursor, &header, ".upd_info").is_none());
    }
}
