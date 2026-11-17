//! Blocking HTTPS downloads to a temp file. Used by the CLI/GUI when the
//! install target is a URL instead of a local file. Kept blocking (rather
//! than async) so pallet-core has no required async runtime; the daemon,
//! which is async, calls into this via `spawn_blocking`.

use crate::error::{Error, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct DownloadedFile {
    pub path: PathBuf,
    pub content_type: Option<String>,
}

/// Download `url` into `dest_dir`, naming the file after the URL's last path
/// segment (falling back to a generic name). Returns the full path written.
pub fn download(url: &str, dest_dir: &Path) -> Result<DownloadedFile> {
    std::fs::create_dir_all(dest_dir)?;
    let parsed = url::Url::parse(url).map_err(|e| Error::other(format!("invalid URL: {e}")))?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(Error::other("only http:// and https:// URLs are supported"));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(concat!("pallet/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let mut response = client.get(parsed.clone()).send()?.error_for_status()?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let file_name = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download.bin");
    let safe_name = sanitize_file_name(file_name);
    let dest_path = dest_dir.join(&safe_name);

    let mut out = std::fs::File::create(&dest_path)?;
    let mut buf = [0u8; 1 << 16];
    use std::io::Read;
    loop {
        let n = response.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
    }
    out.flush()?;

    Ok(DownloadedFile {
        path: dest_path,
        content_type,
    })
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "download.bin".to_string()
    } else {
        cleaned
    }
}
