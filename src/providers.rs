use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ProviderId {
    Claude = 0,
    Codex = 1,
    Antigravity = 2,
    #[serde(rename = "opencode")]
    OpenCode = 3,
    Cursor = 4,
    Grok = 5,
    Copilot = 6,
}
impl ProviderId {
    pub const ALL: [Self; 7] = [
        Self::Claude,
        Self::Codex,
        Self::Antigravity,
        Self::OpenCode,
        Self::Cursor,
        Self::Grok,
        Self::Copilot,
    ];
    pub const fn key(self) -> &'static str {
        [
            "claude",
            "codex",
            "antigravity",
            "opencode",
            "cursor",
            "grok",
            "copilot",
        ][self as usize]
    }
    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == k)
    }
}
impl Default for ProviderId {
    fn default() -> Self {
        Self::Codex
    }
}
