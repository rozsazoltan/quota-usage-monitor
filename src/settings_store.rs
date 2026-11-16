//! Serialize saves and replace a complete file without truncating the old settings.
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

static SAVES: Mutex<()> = Mutex::new(());
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsPath(pub PathBuf);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsPaths {
    pub current: SettingsPath,
    pub codex_legacy: SettingsPath,
    pub claude_legacy: SettingsPath,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSource {
    Current,
    CodexLegacy,
    ClaudeLegacy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsFormat {
    Empty,
    Versionless,
    Version1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationState {
    None,
    Legacy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fingerprint(pub Option<[u8; 32]>);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationObservation {
    path: SettingsPath,
    fingerprint: Fingerprint,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteAuthorization {
    observations: Vec<AuthorizationObservation>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session<T> {
    target: SettingsPath,
    source: SettingsSource,
    migration: MigrationState,
    baseline: T,
    format: SettingsFormat,
    fingerprint: Fingerprint,
    authorization: WriteAuthorization,
    pending_full_save: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded<T> {
    pub value: T,
    pub format: SettingsFormat,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded<T> {
    pub session: Session<T>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadFailure<T> {
    pub settings: T,
    pub error: String,
    pub blocked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadResult<T> {
    Loaded(Loaded<T>),
    Failed(LoadFailure<T>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveIntent {
    Full,
    PaneOnly,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveOutcome {
    pub fingerprint: Fingerprint,
}
#[derive(Debug)]
pub enum SettingsError {
    Io(io::Error),
    Conflict,
    Unauthorized,
    Encode(serde_json::Error),
    Poisoned,
}

impl From<io::Error> for SettingsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl<T> Session<T> {
    pub fn source(&self) -> SettingsSource {
        self.source
    }
    pub fn migration(&self) -> MigrationState {
        self.migration
    }
    pub fn format(&self) -> SettingsFormat {
        self.format
    }
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }
    pub fn baseline(&self) -> &T {
        &self.baseline
    }
    pub fn target(&self) -> &SettingsPath {
        &self.target
    }
    pub fn pending_full_save(&self) -> bool {
        self.pending_full_save
    }
}

pub fn load<T>(
    paths: &SettingsPaths,
    fresh: impl FnOnce() -> T,
    decode: impl Fn(&[u8]) -> Result<Decoded<T>, String>,
) -> LoadResult<T> {
    let paths = [
        (&paths.current, SettingsSource::Current),
        (&paths.codex_legacy, SettingsSource::CodexLegacy),
        (&paths.claude_legacy, SettingsSource::ClaudeLegacy),
    ];
    let mut observations = Vec::new();
    for (path, source) in paths.iter() {
        let bytes = match std::fs::read(&path.0) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                observations.push(AuthorizationObservation {
                    path: (*path).clone(),
                    fingerprint: Fingerprint(None),
                });
                continue;
            }
            Err(error) => {
                return LoadResult::Failed(LoadFailure {
                    settings: fresh(),
                    error: error.to_string(),
                    blocked: true,
                })
            }
        };
        let fingerprint = Fingerprint(Some(sha256(&bytes)));
        let decoded = match decode(&bytes) {
            Ok(value) => value,
            Err(error) => {
                return LoadResult::Failed(LoadFailure {
                    settings: fresh(),
                    error,
                    blocked: true,
                })
            }
        };
        observations.push(AuthorizationObservation {
            path: (*path).clone(),
            fingerprint: fingerprint.clone(),
        });
        let session = Session {
            target: paths[0].0.clone(),
            source: *source,
            migration: if *source != SettingsSource::Current {
                MigrationState::Legacy
            } else if decoded.format == SettingsFormat::Versionless {
                MigrationState::Legacy
            } else {
                MigrationState::None
            },
            baseline: decoded.value,
            format: decoded.format,
            fingerprint,
            authorization: WriteAuthorization { observations },
            pending_full_save: false,
        };
        return LoadResult::Loaded(Loaded { session });
    }
    let baseline = fresh();
    let session = Session {
        target: paths[0].0.clone(),
        source: SettingsSource::Current,
        migration: MigrationState::None,
        baseline,
        format: SettingsFormat::Empty,
        fingerprint: Fingerprint(None),
        authorization: WriteAuthorization { observations },
        pending_full_save: false,
    };
    LoadResult::Loaded(Loaded { session })
}

pub fn fingerprint(path: &Path) -> io::Result<Fingerprint> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Fingerprint(Some(sha256(&bytes)))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Fingerprint(None)),
        Err(error) => Err(error),
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}

pub fn save_guarded<T: Serialize>(
    session: &mut Session<T>,
    intent: SaveIntent,
    snapshot: impl FnOnce(&T) -> T,
) -> Result<SaveOutcome, SettingsError> {
    if intent == SaveIntent::Full {
        session.pending_full_save = true;
    }
    let _save = SAVES.lock().map_err(|_| SettingsError::Poisoned)?;
    if session.authorization.observations.is_empty() {
        return Err(SettingsError::Unauthorized);
    }
    let value = snapshot(&session.baseline);
    let bytes = match serde_json::to_vec_pretty(&value) {
        Ok(bytes) => bytes,
        Err(error) => {
            if intent == SaveIntent::Full {
                session.pending_full_save = true;
            }
            return Err(SettingsError::Encode(error));
        }
    };
    let observations = &session.authorization.observations;
    if let Err(error) = write_complete_guarded(&session.target.0, &bytes, || {
        for observation in observations {
            let actual = fingerprint(&observation.path.0)?;
            if actual != observation.fingerprint {
                return Err(SettingsError::Conflict);
            }
        }
        Ok(())
    }) {
        if intent == SaveIntent::Full {
            session.pending_full_save = true;
        }
        return Err(error);
    }
    let next = Fingerprint(Some(sha256(&bytes)));
    session.baseline = value;
    session.fingerprint = next.clone();
    if let Some(target) = session
        .authorization
        .observations
        .iter_mut()
        .find(|o| o.path == session.target)
    {
        target.fingerprint = next.clone();
    } else {
        session
            .authorization
            .observations
            .push(AuthorizationObservation {
                path: session.target.clone(),
                fingerprint: next.clone(),
            });
    }
    if intent == SaveIntent::Full {
        session.pending_full_save = false;
    }
    Ok(SaveOutcome { fingerprint: next })
}

/// Take the snapshot after acquiring the write lock, so an older save cannot
/// overtake a newer UI change. The app-state lock is released before disk I/O.
#[cfg(test)]
pub fn save<T: Serialize>(path: &Path, snapshot: impl FnOnce() -> Option<T>) -> io::Result<()> {
    let _save = SAVES.lock().unwrap_or_else(|e| e.into_inner());
    let Some(value) = snapshot() else {
        return Ok(());
    };
    let json = serde_json::to_vec_pretty(&value).map_err(io::Error::other)?;
    write_complete(path, &json)
}

#[cfg_attr(not(test), allow(dead_code))]
fn write_complete(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_complete_guarded(path, bytes, || Ok(()))
}

fn write_complete_guarded<E>(
    path: &Path,
    bytes: &[u8],
    before_replace: impl FnOnce() -> Result<(), E>,
) -> Result<(), E>
where
    E: From<io::Error>,
{
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("settings path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("settings path has no filename"))?;
    let temporary = parent.join(format!(
        ".{}.{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        before_replace()?;
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(io::Error::other)
        .map_err(E::from)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    #[test]
    fn replacement_failure_preserves_old_settings_and_recovery_replaces_them() {
        let directory = std::env::temp_dir().join(format!("quota-settings-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        save(&path, || Some(serde_json::json!({"interval": 900000}))).unwrap();
        let old = std::fs::read(&path).unwrap();
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(save(&path, || Some(serde_json::json!({"interval": 60000}))).is_err());
        drop(locked);
        assert_eq!(std::fs::read(&path).unwrap(), old);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        save(&path, || Some(serde_json::json!({"interval": 60000}))).unwrap();
        let new: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(new["interval"], 60000);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
