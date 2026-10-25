use crate::{accounts::AccountKey, providers::ProviderId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decimal {
    pub coefficient: i128,
    pub scale: u32,
}

impl Decimal {
    pub fn parse(value: &str) -> Result<Self, String> {
        let (negative, unsigned) = value
            .strip_prefix('-')
            .map_or((false, value), |v| (true, v));
        let mut parts = unsigned.split('.');
        let whole = parts.next().unwrap_or_default();
        let fraction = parts.next();
        if parts.next().is_some()
            || whole.is_empty()
            || (whole.len() > 1 && whole.starts_with('0'))
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || fraction.is_some_and(|f| {
                f.is_empty() || f.len() > 18 || !f.bytes().all(|b| b.is_ascii_digit())
            })
        {
            return Err("Invalid decimal".into());
        }
        let fraction = fraction.unwrap_or_default();
        let mut coefficient = format!("{whole}{fraction}")
            .parse::<i128>()
            .map_err(|_| "Decimal overflow")?;
        if negative {
            coefficient = coefficient.checked_neg().ok_or("Decimal overflow")?;
        }
        Ok(Self {
            coefficient,
            scale: fraction.len() as u32,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaWindow {
    Session,
    Weekly,
    Monthly,
    Rolling { seconds: u64 },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowMetadata {
    pub window: QuotaWindow,
    pub label: Option<String>,
    pub resets_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PercentageValue {
    Missing,
    Known(f64),
    LegacyApproximate(f64),
}

impl PercentageValue {
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Missing => true,
            Self::Known(value) | Self::LegacyApproximate(value) => {
                value.is_finite() && *value >= 0.0
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuantityValue {
    Missing,
    Unlimited,
    Limited(Decimal),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageLimit {
    pub key: String,
    pub kind: String,
    pub label: String,
    pub scope: Option<serde_json::Value>,
    pub window: Option<WindowMetadata>,
    pub quantity: QuantityValue,
    pub percentage: PercentageValue,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Freshness {
    pub stale: bool,
    pub observed_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageGroup {
    pub key: String,
    pub limits: Vec<UsageLimit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditBalance {
    pub amount: Decimal,
    pub currency_or_unit: String,
    pub on_demand: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreditState {
    Missing,
    Disabled,
    Unlimited,
    Reported(CreditBalance),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub account: AccountKey,
    pub groups: Vec<UsageGroup>,
    pub freshness: Freshness,
    pub credits: CreditState,
}

impl UsageSnapshot {
    pub fn scope_hash(value: &serde_json::Value) -> Result<String, String> {
        use sha2::{Digest, Sha256};

        fn canonical(value: &serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Object(object) => {
                    let sorted = object
                        .iter()
                        .map(|(key, value)| (key.clone(), canonical(value)))
                        .collect();
                    serde_json::Value::Object(sorted)
                }
                serde_json::Value::Array(array) => {
                    serde_json::Value::Array(array.iter().map(canonical).collect())
                }
                other => other.clone(),
            }
        }

        let bytes = serde_json::to_vec(&canonical(value)).map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn validate_percentages(&self) -> bool {
        self.groups
            .iter()
            .flat_map(|group| &group.limits)
            .all(|limit| limit.percentage.is_valid())
    }
}

pub type ProviderSnapshots = BTreeMap<ProviderId, UsageSnapshot>;
