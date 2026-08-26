use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::io::Write;

use serde::{Deserialize, Serialize};

use crate::{
    config::AppDirs,
    error::{Error, Result},
};

const KEYRING_SERVICE: &str = "mqx";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Mqtt,
    Mqtts,
    Ws,
    Wss,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mqtt => "mqtt",
            Self::Mqtts => "mqtts",
            Self::Ws => "ws",
            Self::Wss => "wss",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsConfig {
    #[serde(default = "defaults::validate")]
    pub validate: bool,
    #[serde(default)]
    pub ca_cert_path: Option<PathBuf>,
    #[serde(default)]
    pub client_cert_path: Option<PathBuf>,
    #[serde(default)]
    pub client_key_path: Option<PathBuf>,
    #[serde(default)]
    pub alpn: Option<Vec<String>>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            validate: defaults::validate(),
            ca_cert_path: None,
            client_cert_path: None,
            client_key_path: None,
            alpn: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    #[serde(default)]
    pub clean: bool,
    #[serde(default = "defaults::keep_alive_secs")]
    pub keep_alive_secs: u64,
    #[serde(default = "defaults::max_packet_size")]
    pub max_packet_size: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            clean: false,
            keep_alive_secs: defaults::keep_alive_secs(),
            max_packet_size: defaults::max_packet_size(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub topic: String,
    #[serde(default)]
    pub qos: u8,
}

impl Default for Subscription {
    fn default() -> Self {
        Self {
            topic: "#".into(),
            qos: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastWill {
    pub topic: String,
    pub payload: String,
    #[serde(default)]
    pub qos: u8,
    #[serde(default)]
    pub retain: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
    #[serde(default)]
    pub last_will: Option<LastWill>,
    /// WS/WSS request path. Empty/None → `/mqtt`.
    #[serde(default)]
    pub websocket_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileView {
    pub profile: ConnectionProfile,
    pub has_password: bool,
}

#[derive(Clone, Debug)]
pub struct ProfileStore {
    dir: PathBuf,
    use_keyring: bool,
}

impl ProfileStore {
    pub fn open() -> Result<Self> {
        Self::open_in(AppDirs::new()?.config_dir)
    }

    pub fn open_in(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            use_keyring: keyring_available(),
        })
    }

    pub fn list(&self) -> Result<Vec<ProfileSummary>> {
        let mut profiles = self
            .load_all()?
            .into_iter()
            .map(|profile| ProfileSummary {
                id: profile.id,
                name: profile.name,
                protocol: profile.protocol,
                host: profile.host,
                port: profile.port,
            })
            .collect::<Vec<_>>();
        profiles.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        Ok(profiles)
    }

    pub fn get(&self, id: &str) -> Result<Option<ProfileView>> {
        let Some(profile) = self.load_all()?.into_iter().find(|p| p.id == id) else {
            return Ok(None);
        };
        let has_password = self.secret(id)?.is_some();
        Ok(Some(ProfileView {
            profile,
            has_password,
        }))
    }

    pub fn save(&self, mut profile: ConnectionProfile, password: Option<String>) -> Result<String> {
        if profile.id.is_empty() {
            profile.id = uuid::Uuid::new_v4().to_string();
        } else if !is_safe_profile_id(&profile.id) {
            return Err(Error::InvalidProfileId(profile.id));
        }
        if profile.client_id.is_empty() {
            let suffix = profile.id.get(..8).unwrap_or(profile.id.as_str());
            profile.client_id = format!("mqx-{suffix}");
        }
        if profile.subscriptions.is_empty() {
            profile.subscriptions.push(Subscription::default());
        }

        let mut profiles = self.load_all()?;
        if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile.clone();
        } else {
            profiles.push(profile.clone());
        }
        self.write_all(&profiles)?;

        if let Some(password) = password {
            if password.is_empty() {
                self.delete_secret(&profile.id)?;
            } else {
                self.set_secret(&profile.id, &password)?;
            }
        }
        Ok(profile.id)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut profiles = self.load_all()?;
        let before = profiles.len();
        profiles.retain(|p| p.id != id);
        if profiles.len() == before {
            return Err(Error::ProfileNotFound(id.into()));
        }
        self.write_all(&profiles)?;
        if is_safe_profile_id(id) {
            self.delete_secret(id)?;
        }
        Ok(())
    }

    pub fn secret(&self, id: &str) -> Result<Option<String>> {
        if self.use_keyring
            && let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, id)
        {
            match entry.get_password() {
                Ok(password) => return Ok(Some(password)),
                Err(keyring::Error::NoEntry) => {}
                Err(_) => {}
            }
        }
        self.read_secret_file(id)
    }

    fn set_secret(&self, id: &str, password: &str) -> Result<()> {
        if self.use_keyring
            && let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, id)
            && entry.set_password(password).is_ok()
        {
            let _ = self.remove_secret_file(id);
            return Ok(());
        }
        // Headless Linux often has no Secret Service; keep the secret out of connections.json.
        self.write_secret_file(id, password)
    }

    fn delete_secret(&self, id: &str) -> Result<()> {
        if self.use_keyring
            && let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, id)
        {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => {}
            }
        }
        self.remove_secret_file(id)
    }

    fn connections_path(&self) -> PathBuf {
        self.dir.join("connections.json")
    }

    fn secrets_dir(&self) -> PathBuf {
        self.dir.join("secrets")
    }

    fn secret_path(&self, id: &str) -> Result<PathBuf> {
        if !is_safe_profile_id(id) {
            return Err(Error::InvalidProfileId(id.into()));
        }
        let dir = self.secrets_dir();
        let path = dir.join(id);
        if path.parent() != Some(dir.as_path()) {
            return Err(Error::InvalidProfileId(id.into()));
        }
        Ok(path)
    }

    fn load_all(&self) -> Result<Vec<ConnectionProfile>> {
        let path = self.connections_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&path)?;
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_str(&content).map_err(|source| Error::Json { path, source })
    }

    fn write_all(&self, profiles: &[ConnectionProfile]) -> Result<()> {
        let path = self.connections_path();
        let tmp = path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(profiles).map_err(|source| Error::Json {
                path: path.clone(),
                source,
            })?,
        )?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    fn read_secret_file(&self, id: &str) -> Result<Option<String>> {
        let path = self.secret_path(id)?;
        if !path.exists() {
            return Ok(None);
        }
        let contents = fs::read_to_string(path)?;
        if contents.is_empty() {
            Ok(None)
        } else {
            Ok(Some(contents))
        }
    }

    fn write_secret_file(&self, id: &str, password: &str) -> Result<()> {
        self.ensure_secrets_dir()?;
        let path = self.secret_path(id)?;
        write_private_file(&path, password.as_bytes())
    }

    fn ensure_secrets_dir(&self) -> Result<PathBuf> {
        let dir = self.secrets_dir();
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(dir)
    }

    fn remove_secret_file(&self, id: &str) -> Result<()> {
        let Ok(path) = self.secret_path(id) else {
            return Ok(());
        };
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

fn keyring_available() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    ))
}

fn is_safe_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(contents).map_err(Error::from)?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, contents)?;
    }
    Ok(())
}

mod defaults {
    pub fn validate() -> bool {
        true
    }

    pub fn keep_alive_secs() -> u64 {
        60
    }

    pub fn max_packet_size() -> u32 {
        10_000_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl ProfileStore {
        fn isolated(dir: impl Into<PathBuf>) -> Self {
            Self {
                dir: dir.into(),
                use_keyring: false,
            }
        }
    }

    fn sample(id: &str, name: &str) -> ConnectionProfile {
        ConnectionProfile {
            id: id.into(),
            name: name.into(),
            protocol: Protocol::Mqtts,
            host: "ha.local".into(),
            port: 8883,
            client_id: String::new(),
            username: "hauke".into(),
            tls: TlsConfig::default(),
            session: SessionConfig::default(),
            subscriptions: Vec::new(),
            last_will: None,
            websocket_path: None,
        }
    }

    #[test]
    fn save_get_list_delete_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::isolated(dir.path());
        let id = store
            .save(sample("", "Home Assistant"), Some("secret".into()))
            .unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Home Assistant");
        assert_eq!(listed[0].protocol, Protocol::Mqtts);

        let view = store.get(&id).unwrap().unwrap();
        assert!(view.has_password);
        assert_eq!(view.profile.username, "hauke");
        assert_eq!(view.profile.client_id, format!("mqx-{}", &id[..8]));
        assert_eq!(view.profile.subscriptions[0].topic, "#");
        assert_eq!(store.secret(&id).unwrap().as_deref(), Some("secret"));

        store.delete(&id).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(store.get(&id).unwrap().is_none());
        assert!(store.secret(&id).unwrap().is_none());
    }

    #[test]
    fn password_stays_out_of_connections_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::isolated(dir.path());
        store
            .save(sample("abc", "Local"), Some("pw".into()))
            .unwrap();
        let raw = fs::read_to_string(dir.path().join("connections.json")).unwrap();
        assert!(!raw.contains("pw"));
        assert!(dir.path().join("secrets").join("abc").exists());
    }

    #[test]
    fn rejects_path_escape_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::isolated(dir.path());
        store
            .save(sample("safe", "Safe"), Some("keep".into()))
            .unwrap();
        let connections = dir.path().join("connections.json");
        let before = fs::read_to_string(&connections).unwrap();

        let err = store
            .save(sample("../connections.json", "Evil"), Some("pwned".into()))
            .unwrap_err();
        assert!(matches!(err, Error::InvalidProfileId(_)));
        let after = fs::read_to_string(&connections).unwrap();
        assert_eq!(before, after);
        assert!(!after.contains("pwned"));
        assert!(store.secret("../connections.json").is_err());

        let _ = store.delete("../connections.json");
        assert!(connections.exists());
        assert_eq!(fs::read_to_string(&connections).unwrap(), after);
    }

    #[cfg(unix)]
    #[test]
    fn secret_dir_and_file_are_private_after_write_and_overwrite() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::isolated(dir.path());
        let secrets = dir.path().join("secrets");
        fs::create_dir_all(&secrets).unwrap();
        fs::set_permissions(&secrets, fs::Permissions::from_mode(0o755)).unwrap();
        let path = secrets.join("abc");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        store
            .save(sample("abc", "Local"), Some("pw".into()))
            .unwrap();
        assert_eq!(
            fs::metadata(&secrets).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        store
            .save(sample("abc", "Local"), Some("pw2".into()))
            .unwrap();
        assert_eq!(
            fs::metadata(&secrets).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(store.secret("abc").unwrap().as_deref(), Some("pw2"));
    }
}
