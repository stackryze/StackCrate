//! `.desktop` launcher generation for manually installed AppImages, archives,
//! and local .deb apps that don't already ship their own launcher.

use crate::config::Paths;
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

pub fn slugify(value: &str) -> Result<String> {
    let mut slug = String::new();
    let mut last_was_sep = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
            slug.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('-');
            last_was_sep = true;
        }
    }
    let trimmed = slug.trim_matches(|c| c == '-' || c == '.').to_string();
    if trimmed.is_empty() {
        return Err(Error::other("name must not be empty"));
    }
    Ok(trimmed.chars().take(80).collect())
}

fn desktop_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', " ")
        .replace('"', "\\\"")
}

/// Copy an icon file into the user icon theme directory, named after `slug`.
pub fn install_icon(icon_path: &Path, slug: &str) -> Result<PathBuf> {
    let ext = icon_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["png", "svg", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
        return Err(Error::other("icon must be PNG, SVG, JPG or WEBP"));
    }
    std::fs::create_dir_all(Paths::icon_dir())?;
    let target = Paths::icon_dir().join(format!("{slug}.{ext}"));
    std::fs::copy(icon_path, &target)?;
    Ok(target)
}

/// Write a `.desktop` launcher for `name`, executing `exec_path`, with an
/// optional icon path (already installed via `install_icon` or pointing at a
/// theme icon name). Returns the path written.
pub fn create_desktop_file(
    slug: &str,
    name: &str,
    exec_path: &Path,
    icon: Option<&str>,
    terminal: bool,
) -> Result<PathBuf> {
    std::fs::create_dir_all(Paths::applications_dir())?;
    let desktop_path = Paths::applications_dir().join(format!("{slug}.desktop"));
    let icon_line = icon
        .map(|i| format!("Icon={}\n", desktop_escape(i)))
        .unwrap_or_default();
    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={}\n\
         Exec={} %U\n\
         {}\
         Terminal={}\n\
         Categories=Utility;Application;\n\
         StartupNotify=true\n\
         X-StackCrate-Managed=true\n",
        desktop_escape(name),
        desktop_escape(&exec_path.to_string_lossy()),
        icon_line,
        terminal,
    );
    std::fs::write(&desktop_path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&desktop_path, std::fs::Permissions::from_mode(0o755))?;
    }
    if let Some(bin) = crate::process::which("update-desktop-database") {
        let _ = crate::process::run_owned(
            &bin.to_string_lossy(),
            &[Paths::applications_dir().to_string_lossy().to_string()],
            std::time::Duration::from_secs(10),
        );
    }
    Ok(desktop_path)
}

pub fn remove_desktop_file(slug: &str) -> Result<()> {
    let path = Paths::applications_dir().join(format!("{slug}.desktop"));
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_replaces_separators() {
        assert_eq!(slugify("My Cool App").unwrap(), "my-cool-app");
        assert_eq!(slugify("Vim_Editor-9.1").unwrap(), "vim_editor-9.1");
    }

    #[test]
    fn slugify_collapses_runs_of_separators() {
        assert_eq!(slugify("foo   bar///baz").unwrap(), "foo-bar-baz");
    }

    #[test]
    fn slugify_rejects_empty_input() {
        assert!(slugify("   ").is_err());
        assert!(slugify("!!!").is_err());
    }

    #[test]
    fn slugify_truncates_long_input() {
        let long = "a".repeat(200);
        assert_eq!(slugify(&long).unwrap().len(), 80);
    }
}
