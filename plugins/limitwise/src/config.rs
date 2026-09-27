use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const FIVE_HOUR_RESERVE_PERCENT: f64 = 10.0;
pub const MISSED_GRACE_SECONDS: i64 = 300;
pub const DEFAULT_POLL_SECONDS: u64 = 15;

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub logs_dir: PathBuf,
    pub installed_binary: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self, String> {
        let home = home_dir()?;
        let data_dir = if cfg!(target_os = "macos") {
            home.join("Library")
                .join("Application Support")
                .join("LimitWise")
        } else if let Ok(value) = env::var("XDG_DATA_HOME") {
            PathBuf::from(value).join("limitwise")
        } else {
            home.join(".local").join("share").join("limitwise")
        };
        Ok(Self {
            database: data_dir.join("limitwise.sqlite3"),
            logs_dir: data_dir.join("logs"),
            installed_binary: data_dir.join("bin").join("limitwise"),
            data_dir,
        })
    }

    pub fn ensure(&self) -> Result<(), String> {
        fs::create_dir_all(&self.logs_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(
            self.installed_binary
                .parent()
                .ok_or_else(|| "invalid installed binary path".to_string())?,
        )
        .map_err(|e| e.to_string())?;
        set_private_dir(&self.data_dir)?;
        set_private_dir(&self.logs_dir)?;
        Ok(())
    }
}

pub fn home_dir() -> Result<PathBuf, String> {
    env::var_os("LIMITWISE_HOME")
        .or_else(|| env::var_os("HOME"))
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "LIMITWISE_HOME, HOME, or USERPROFILE is required".to_string())
}

pub fn poll_seconds() -> u64 {
    env::var("LIMITWISE_POLL_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_POLL_SECONDS)
}

pub fn codex_binary() -> PathBuf {
    let override_path = env::var_os("LIMITWISE_CODEX_PATH").map(PathBuf::from);
    let search_path = env::var_os("PATH");
    let home = home_dir().ok();
    discover_codex_binary(override_path, search_path.as_deref(), home.as_deref())
}

fn discover_codex_binary(
    override_path: Option<PathBuf>,
    search_path: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
) -> PathBuf {
    if let Some(path) = override_path {
        return path;
    }
    if let Some(path) = search_path.and_then(find_binary_on_path) {
        return path;
    }
    if let Some(home) = home {
        for candidate in [
            home.join(".local/bin/codex"),
            home.join(".codex/packages/standalone/current/bin/codex"),
        ] {
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("codex")
}

fn find_binary_on_path(path: &std::ffi::OsStr) -> Option<PathBuf> {
    env::split_paths(path)
        .map(|directory| directory.join("codex"))
        .find(|candidate| candidate.is_file())
}

pub fn system_timezone() -> String {
    if let Ok(value) = env::var("TZ") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    if let Ok(target) = fs::read_link("/etc/localtime") {
        let rendered = target.to_string_lossy();
        if let Some((_, zone)) = rendered.split_once("zoneinfo/") {
            return zone.to_string();
        }
    }
    "UTC".to_string()
}

#[cfg(unix)]
pub fn set_private_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
pub fn set_private_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
pub fn set_private_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
pub fn set_private_dir(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn finds_codex_in_an_explicit_path_without_shell_startup_files() {
        let root = env::temp_dir().join(format!(
            "limitwise-codex-path-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("codex");
        fs::write(&binary, b"fixture").unwrap();

        assert_eq!(find_binary_on_path(root.as_os_str()), Some(binary));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn falls_back_to_user_local_codex_when_service_path_is_minimal() {
        let root = env::temp_dir().join(format!(
            "limitwise-codex-home-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let local_bin = root.join(".local/bin");
        fs::create_dir_all(&local_bin).unwrap();
        let binary = local_bin.join("codex");
        fs::write(&binary, b"fixture").unwrap();

        assert_eq!(
            discover_codex_binary(None, Some(std::ffi::OsStr::new("/usr/bin")), Some(&root)),
            binary
        );
        fs::remove_dir_all(root).unwrap();
    }
}
