use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use serde::Deserialize;
use serde::Serialize;

/// Everything needed to reach a server, including the password.
///
/// The password is stored in plaintext, deliberately. Subsonic auth is
/// `md5(password + salt)` with a fresh salt per request, so the real password
/// has to be recoverable at runtime — a hash would not do. The file is written
/// `0600`, which is the same bargain most terminal clients make.
///
/// If you would rather it not sit on disk, set `NAVIDROME_PASSWORD`; that wins
/// over the file and nothing is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub url: String,
    pub username: String,
    #[serde(default)]
    pub password: String,
}

/// Overrides the stored password when set, for people who keep secrets in a
/// manager and inject them into the environment.
pub const PASSWORD_ENV: &str = "NAVIDROME_PASSWORD";

impl ServerConfig {
    pub fn path() -> anyhow::Result<PathBuf> {
        let dir = dirs::config_dir().context("no config directory for this platform")?;
        Ok(dir.join("navidrome-rs").join("config.json"))
    }

    /// Loads the saved server, or `None` if the user has not signed in yet.
    ///
    /// A corrupt config is treated as "not signed in" rather than a hard error:
    /// the sign-in screen can always recreate it.
    pub fn load() -> Option<Self> {
        let path = Self::path().ok()?;
        let bytes = std::fs::read(&path).ok()?;
        match serde_json::from_slice(&bytes) {
            Ok(config) => Some(config),
            Err(err) => {
                log::warn!("ignoring unreadable config at {}: {err}", path.display());
                None
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
        restrict(&path)?;
        Ok(())
    }

    /// The password to authenticate with: the environment wins, then the file.
    pub fn password(&self) -> String {
        std::env::var(PASSWORD_ENV).unwrap_or_else(|_| self.password.clone())
    }

}

/// Tightens permissions to owner-only, since the file holds a password.
#[cfg(unix)]
fn restrict(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restricting {}", path.display()))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

/// Where cached audio lives, and how much of it we keep.
pub fn cache_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::cache_dir().context("no cache directory for this platform")?;
    Ok(dir.join("navidrome-rs").join("tracks"))
}

/// 2 GiB, as requested. Original files are cached as-is, so this holds fewer
/// tracks than a transcoded cache would, in exchange for bit-perfect playback.
pub const CACHE_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn config_roundtrips_including_the_password() {
        let config = ServerConfig {
            url: "https://patskovn-pi".into(),
            username: "nikita".into(),
            password: "hunter2".into(),
        };
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<ServerConfig>(&json).unwrap(), config);
    }

    /// Configs written before the password moved into the file must still load.
    #[test]
    fn an_older_config_without_a_password_still_parses() {
        let json = r#"{"url":"https://x","username":"nikita"}"#;
        let config: ServerConfig = serde_json::from_str(json).unwrap();
        assert!(config.password.is_empty());
    }

    #[test]
    fn the_environment_overrides_the_stored_password() {
        let config = ServerConfig {
            url: "https://x".into(),
            username: "nikita".into(),
            password: "from-file".into(),
        };
        assert_eq!(config.password(), "from-file");

        // SAFETY: single-threaded test, and the variable is removed below.
        unsafe { std::env::set_var(PASSWORD_ENV, "from-env") };
        assert_eq!(config.password(), "from-env");
        unsafe { std::env::remove_var(PASSWORD_ENV) };
    }

    #[cfg(unix)]
    #[test]
    fn a_saved_config_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("navidrome-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, b"{}").unwrap();
        restrict(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "config should be owner-only, got {mode:o}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
