use crate::{
    accounts::AccountKey,
    models::{AppUsageData, UsageData, UsageSection},
    providers::ProviderId,
    usage_model::{
        CreditState, PercentageValue, QuantityValue, QuotaWindow, UsageGroup, UsageLimit,
        UsageSnapshot, WindowMetadata,
    },
};
use std::time::{Duration, UNIX_EPOCH};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LegacyPresence {
    Present,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionLoss {
    GroupStructure,
    Credits,
    UnsupportedQuantity,
    UnsupportedWindow,
    AmbiguousWindow,
    MissingPercentage,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyProvenance {
    pub session: LegacyPresence,
    pub weekly: LegacyPresence,
    pub approximate_legacy_percentages: bool,
    pub losses: Vec<ProjectionLoss>,
}

pub fn from_legacy(provider: ProviderId, data: &UsageData, account: AccountKey) -> UsageSnapshot {
    let legacy_limit = |window, section: &UsageSection, key: &str| UsageLimit {
        key: key.to_owned(),
        kind: "quota".to_owned(),
        label: key.to_owned(),
        scope: None,
        window: Some(WindowMetadata {
            window,
            label: None,
            resets_at: section
                .resets_at
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs()),
        }),
        quantity: QuantityValue::Missing,
        percentage: PercentageValue::LegacyApproximate(section.percentage),
    };
    let mut limits = vec![
        legacy_limit(QuotaWindow::Session, &data.session, "session"),
        legacy_limit(QuotaWindow::Weekly, &data.weekly, "weekly"),
    ];
    if account.provider != provider {
        limits.clear();
    }
    UsageSnapshot {
        account,
        groups: vec![UsageGroup {
            key: "legacy".to_owned(),
            limits,
        }],
        freshness: Default::default(),
        credits: CreditState::Missing,
    }
}

pub fn project_legacy(
    snapshot: &UsageSnapshot,
    bindings: &[AccountKey],
) -> (AppUsageData, LegacyProvenance) {
    let mut app = AppUsageData::default();
    let mut provenance = LegacyProvenance::default();
    if !bindings.contains(&snapshot.account) {
        return (app, provenance);
    }
    let session = project_window(snapshot, QuotaWindow::Session, &mut provenance);
    let weekly = project_window(snapshot, QuotaWindow::Weekly, &mut provenance);
    let data = UsageData { session, weekly };
    match snapshot.account.provider {
        ProviderId::Claude => app.claude_code = Some(data),
        ProviderId::Codex => app.codex = Some(data),
        ProviderId::Antigravity => app.antigravity = Some(data),
        ProviderId::OpenCode => app.opencode = Some(data),
        ProviderId::Cursor => app.cursor = Some(data),
        ProviderId::Grok => app.grok = Some(data),
        ProviderId::Copilot => app.copilot = Some(data),
    }
    if !snapshot.groups.is_empty() {
        provenance.losses.push(ProjectionLoss::GroupStructure);
    }
    if snapshot.credits != CreditState::Missing {
        provenance.losses.push(ProjectionLoss::Credits);
    }
    provenance.approximate_legacy_percentages = snapshot
        .groups
        .iter()
        .flat_map(|group| &group.limits)
        .any(|limit| matches!(limit.percentage, PercentageValue::LegacyApproximate(_)));
    (app, provenance)
}

fn project_window(
    snapshot: &UsageSnapshot,
    window: QuotaWindow,
    provenance: &mut LegacyProvenance,
) -> UsageSection {
    let matching: Vec<_> = snapshot
        .groups
        .iter()
        .flat_map(|group| &group.limits)
        .filter(|limit| {
            limit
                .window
                .as_ref()
                .is_some_and(|metadata| metadata.window == window)
        })
        .collect();
    if matching.len() > 1 {
        provenance.losses.push(ProjectionLoss::AmbiguousWindow);
    }
    let Some(limit) = matching.first() else {
        provenance.losses.push(ProjectionLoss::MissingPercentage);
        return UsageSection::default();
    };
    if !matches!(limit.quantity, QuantityValue::Missing) {
        provenance.losses.push(ProjectionLoss::UnsupportedQuantity);
    }
    let mut section = UsageSection::default();
    match limit.percentage {
        PercentageValue::Known(value) | PercentageValue::LegacyApproximate(value) => {
            section.percentage = value
        }
        PercentageValue::Missing => provenance.losses.push(ProjectionLoss::MissingPercentage),
    }
    if let Some(metadata) = &limit.window {
        if !matches!(metadata.window, QuotaWindow::Session | QuotaWindow::Weekly) {
            provenance.losses.push(ProjectionLoss::UnsupportedWindow);
        }
        section.resets_at = metadata
            .resets_at
            .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)));
    }
    if window == QuotaWindow::Session {
        provenance.session = LegacyPresence::Present;
    }
    if window == QuotaWindow::Weekly {
        provenance.weekly = LegacyPresence::Present;
    }
    section
}
