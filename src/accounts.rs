use crate::providers::ProviderId;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct AccountId(String);
impl<'de> Deserialize<'de> for AccountId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}
impl AccountId {
    pub fn new(s: impl Into<String>) -> Result<Self, String> {
        let s = s.into();
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err("Account ID must contain ASCII letters, digits, or underscores".into());
        }
        Ok(Self(s))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AccountKey {
    pub provider: ProviderId,
    pub account: AccountId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CredentialReference(String);
impl<'de> Deserialize<'de> for CredentialReference {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        let path = Path::new(&value);
        if path.is_absolute() || value == "~" || value.starts_with("~/") || value.starts_with("~\\")
        {
            Ok(Self(value))
        } else {
            Err(serde::de::Error::custom(
                "Credential path must be absolute or home-relative",
            ))
        }
    }
}
impl CredentialReference {
    pub fn validate(value: impl Into<String>, home: &Path) -> Result<Self, String> {
        let s = value.into();
        let p = Path::new(&s);
        if p.is_absolute() || s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
            let reference = Self(s);
            let _ = reference.resolve(home);
            Ok(reference)
        } else {
            Err("Credential path must be absolute or home-relative".into())
        }
    }
    pub fn resolve(&self, home: &Path) -> PathBuf {
        let p = Path::new(&self.0);
        if self.0 == "~" {
            home.to_path_buf()
        } else if let Some(t) = self
            .0
            .strip_prefix("~/")
            .or_else(|| self.0.strip_prefix("~\\"))
        {
            home.join(t)
        } else {
            p.to_path_buf()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProfile {
    pub id: AccountId,
    pub name: String,
    pub config_dir: Option<CredentialReference>,
    pub credentials: Option<CredentialReference>,
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccounts {
    pub profiles: Vec<AccountProfile>,
    pub default: AccountId,
    pub selected: Option<AccountId>,
    pub used_ids: BTreeSet<String>,
}
impl ProviderAccounts {
    pub fn normalize(&mut self) {
        self.used_ids = self
            .used_ids
            .iter()
            .map(|id| id.to_ascii_lowercase())
            .collect();
        let mut reserved = self.used_ids.clone();
        for profile in &self.profiles {
            reserved.insert(profile.id.as_str().to_ascii_lowercase());
        }
        reserved.insert(self.default.as_str().to_ascii_lowercase());
        if let Some(selected) = &self.selected {
            reserved.insert(selected.as_str().to_ascii_lowercase());
        }
        let mut seen = BTreeSet::new();
        for profile in &mut self.profiles {
            let key = profile.id.as_str().to_ascii_lowercase();
            if !seen.insert(key) {
                let mut index = 1;
                loop {
                    let candidate = format!("account_{index}");
                    index += 1;
                    if reserved.insert(candidate.clone()) {
                        profile.id = AccountId(candidate.clone());
                        profile.name = candidate;
                        profile.enabled = false;
                        break;
                    }
                }
            }
            self.used_ids
                .insert(profile.id.as_str().to_ascii_lowercase());
        }
        if !self
            .profiles
            .iter()
            .any(|profile| profile.id == self.default && profile.enabled)
        {
            if let Some(profile) = self.profiles.iter_mut().find(|profile| profile.enabled) {
                self.default = profile.id.clone();
            } else {
                let mut index = 1;
                loop {
                    let candidate = format!("account_{index}");
                    index += 1;
                    if reserved.insert(candidate.clone()) {
                        let id = AccountId(candidate.clone());
                        self.profiles.push(AccountProfile {
                            id: id.clone(),
                            name: candidate,
                            config_dir: None,
                            credentials: None,
                            enabled: true,
                        });
                        self.default = id;
                        break;
                    }
                }
            }
        }
        if self
            .selected
            .as_ref()
            .is_none_or(|id| !self.profiles.iter().any(|p| &p.id == id && p.enabled))
        {
            self.selected = self
                .profiles
                .iter()
                .find(|p| p.enabled)
                .map(|p| p.id.clone());
        }
        for profile in &mut self.profiles {
            if profile.name.trim().is_empty() {
                profile.name = profile.id.as_str().to_owned();
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSettings {
    pub providers: std::collections::BTreeMap<ProviderId, ProviderAccounts>,
}
impl AccountSettings {
    pub fn legacy_automatic() -> Self {
        let providers = [ProviderId::Claude, ProviderId::Codex]
            .into_iter()
            .map(|provider| {
                let id = AccountId::new("default").expect("static account id is valid");
                (
                    provider,
                    ProviderAccounts {
                        profiles: vec![AccountProfile {
                            id: id.clone(),
                            name: "default".into(),
                            config_dir: None,
                            credentials: None,
                            enabled: true,
                        }],
                        default: id.clone(),
                        selected: None,
                        used_ids: ["default".to_owned()].into_iter().collect(),
                    },
                )
            })
            .collect();
        Self { providers }
    }

    pub fn validate_persisted(&self) -> Result<(), String> {
        for (provider, accounts) in &self.providers {
            if !matches!(provider, ProviderId::Claude | ProviderId::Codex) {
                return Err(format!("Unsupported provider: {}", provider.key()));
            }
            let mut ids = BTreeSet::new();
            for profile in &accounts.profiles {
                if !ids.insert(profile.id.as_str().to_ascii_lowercase()) {
                    return Err("Account IDs must be unique without regard to case".into());
                }
                for reference in [&profile.config_dir, &profile.credentials]
                    .into_iter()
                    .flatten()
                {
                    let value = &reference.0;
                    if !Path::new(value).is_absolute()
                        && value != "~"
                        && !value.starts_with("~/")
                        && !value.starts_with("~\\")
                    {
                        return Err("Credential path must be absolute or home-relative".into());
                    }
                }
            }
            if !ids.contains(&accounts.default.as_str().to_ascii_lowercase())
                || !accounts
                    .profiles
                    .iter()
                    .any(|p| p.id == accounts.default && p.enabled)
            {
                return Err("Default account must identify an enabled profile".into());
            }
            if let Some(selected) = &accounts.selected {
                if !accounts
                    .profiles
                    .iter()
                    .any(|p| &p.id == selected && p.enabled)
                {
                    return Err("Selected account must identify an enabled profile".into());
                }
            }
            let mut used_ids = BTreeSet::new();
            if accounts.used_ids.iter().any(|id| {
                AccountId::new(id.clone()).is_err() || !used_ids.insert(id.to_ascii_lowercase())
            }) || accounts.profiles.iter().any(|p| {
                !accounts
                    .used_ids
                    .iter()
                    .any(|id| id.eq_ignore_ascii_case(p.id.as_str()))
            }) || accounts.profiles.iter().any(|p| {
                used_ids
                    .iter()
                    .filter(|id| *id == &p.id.as_str().to_ascii_lowercase())
                    .count()
                    != 1
            }) {
                return Err("Invalid or incomplete used account IDs".into());
            }
        }
        Ok(())
    }

    pub fn normalize(&mut self) {
        self.providers
            .retain(|provider, _| matches!(provider, ProviderId::Claude | ProviderId::Codex));
        for accounts in self.providers.values_mut() {
            accounts.normalize();
        }
    }
}
