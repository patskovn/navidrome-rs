use std::path::PathBuf;

use anyhow::Context;
use serde::Deserialize;
use serde::Serialize;

const KEYRING_SERVICE: &str = "navidrome-rs";

/// Everything needed to reach a server, minus the password.
///
/// The password is deliberately not part of this struct so it cannot be
/// serialised by accident; it lives in the OS keychain and is fetched on demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub url: String,
    pub username: String,
}

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
        Ok(())
    }

    /// Keychain entry key. Scoped by server so several instances can coexist.
    fn keyring_user(&self) -> String {
        format!("{}@{}", self.username, self.url)
    }

    /// Stores the password in the OS keychain.
    ///
    /// Subsonic token auth is `md5(password + salt)` with a fresh salt per
    /// request, so the plaintext password genuinely has to be retrievable — a
    /// stored hash would not be enough. The keychain is the least-bad place for
    /// it, and notably better than the plaintext JSON config this app's sibling
    /// (`tgpt`) uses for its API key.
    pub fn store_password(&self, password: &str) -> anyhow::Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &self.keyring_user())
            .context("opening keychain entry")?;
        entry.set_password(password).context("writing to keychain")?;
        Ok(())
    }

    pub fn password(&self) -> anyhow::Result<String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &self.keyring_user())
            .context("opening keychain entry")?;
        entry.get_password().context("reading from keychain")
    }

    /// Used by a future "sign out" action; kept next to the writer so the
    /// keychain key stays in one place.
    #[allow(dead_code)]
    pub fn forget_password(&self) -> anyhow::Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &self.keyring_user())
            .context("opening keychain entry")?;
        // Deleting a credential that was never stored is not an error here.
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).context("deleting from keychain"),
        }
    }
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
    fn keyring_user_is_scoped_by_server() {
        let a = ServerConfig {
            url: "https://one".into(),
            username: "nikita".into(),
        };
        let b = ServerConfig {
            url: "https://two".into(),
            username: "nikita".into(),
        };
        assert_ne!(a.keyring_user(), b.keyring_user());
    }

    #[test]
    fn config_roundtrips_without_leaking_a_password_field() {
        let config = ServerConfig {
            url: "https://patskovn-pi".into(),
            username: "nikita".into(),
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("password"), "config serialised a password: {json}");
        let back: ServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, back);
    }
}
