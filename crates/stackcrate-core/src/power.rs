//! Whether this machine is currently running on AC power, read straight from
//! the kernel's `/sys/class/power_supply` (no D-Bus/UPower client needed: the
//! same information is always exposed there on any Linux system with a
//! battery, and reading two small files is cheaper and has no extra runtime
//! dependency). Used to gate unattended auto-updates on laptops so a
//! `security-only`/`auto-all` policy doesn't drain a battery mid-call.

use std::path::Path;

/// `Some(true)` on AC power, `Some(false)` on battery, `None` when this
/// machine has no battery at all (desktops, most servers) -- callers should
/// treat `None` the same as `Some(true)`, since "on AC power" is vacuously
/// true when there's no battery to be running down.
pub fn on_ac_power() -> Option<bool> {
    on_ac_power_in(Path::new("/sys/class/power_supply"))
}

fn on_ac_power_in(root: &Path) -> Option<bool> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut saw_battery = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = std::fs::read_to_string(path.join("type"))
            .unwrap_or_default()
            .trim()
            .to_string();
        match kind.as_str() {
            "Battery" => {
                saw_battery = true;
            }
            "Mains" | "USB" if read_online(&path) == Some(true) => {
                return Some(true);
            }
            _ => {}
        }
    }
    if saw_battery { Some(false) } else { None }
}

fn read_online(supply_path: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(supply_path.join("online")).ok()?;
    match text.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, file: &str, contents: &str) {
        let supply_dir = dir.join(name);
        fs::create_dir_all(&supply_dir).unwrap();
        fs::write(supply_dir.join(file), contents).unwrap();
    }

    #[test]
    fn no_power_supply_entries_means_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(on_ac_power_in(tmp.path()), None);
    }

    #[test]
    fn mains_online_means_on_ac_power() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "AC", "type", "Mains\n");
        write(tmp.path(), "AC", "online", "1\n");
        write(tmp.path(), "BAT0", "type", "Battery\n");
        assert_eq!(on_ac_power_in(tmp.path()), Some(true));
    }

    #[test]
    fn mains_offline_with_battery_means_on_battery() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "AC", "type", "Mains\n");
        write(tmp.path(), "AC", "online", "0\n");
        write(tmp.path(), "BAT0", "type", "Battery\n");
        assert_eq!(on_ac_power_in(tmp.path()), Some(false));
    }

    #[test]
    fn battery_only_no_mains_entry_means_on_battery() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "BAT0", "type", "Battery\n");
        assert_eq!(on_ac_power_in(tmp.path()), Some(false));
    }
}
