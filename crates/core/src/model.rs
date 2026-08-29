use serde::{Deserialize, Serialize};

/// A continuous period during which one window was the active (focused)
/// window on the user's desktop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowEvent {
    /// Unix timestamp (seconds) when the window became active.
    pub start: i64,
    /// Unix timestamp (seconds) when it stopped being active.
    pub end: i64,
    /// Normalised application identifier (WM_CLASS / app_id, lowercase).
    pub app_id: String,
    /// Window title at that time.
    pub title: String,
}

impl WindowEvent {
    pub fn new(start: i64, end: i64, app_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            start,
            end,
            app_id: app_id.into(),
            title: title.into(),
        }
    }

    pub fn duration(&self) -> i64 {
        (self.end - self.start).max(0)
    }

    pub fn same_window(&self, app_id: &str, title: &str) -> bool {
        self.app_id == app_id && self.title == title
    }
}

/// A period during which the user was away / the session was locked or idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkSession {
    pub start: i64,
    pub end: i64,
}

impl AfkSession {
    pub fn duration(&self) -> i64 {
        (self.end - self.start).max(0)
    }
}

/// High-level activity categories, in the spirit of Google Digital Wellbeing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Work,
    Browsers,
    Communication,
    Media,
    Creative,
    Gaming,
    System,
    Uncategorised,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::Work,
        Category::Browsers,
        Category::Communication,
        Category::Media,
        Category::Creative,
        Category::Gaming,
        Category::System,
        Category::Uncategorised,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Category::Work => "work",
            Category::Browsers => "browsers",
            Category::Communication => "communication",
            Category::Media => "media",
            Category::Creative => "creative",
            Category::Gaming => "gaming",
            Category::System => "system",
            Category::Uncategorised => "uncategorised",
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            Category::Work => "Work & Coding",
            Category::Browsers => "Browsers",
            Category::Communication => "Communication",
            Category::Media => "Media & Streaming",
            Category::Creative => "Creative & Design",
            Category::Gaming => "Games",
            Category::System => "System & Files",
            Category::Uncategorised => "Uncategorised",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.key() == s)
    }
}

/// Aggregated usage of one application over a time range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppUsage {
    pub app_id: String,
    pub seconds: i64,
    pub sessions: i64,
}

/// Aggregated usage of one window title (documents, sites, projects...) for
/// one application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleUsage {
    pub title: String,
    pub seconds: i64,
    pub sessions: i64,
}

/// Aggregated usage of one category over a time range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryUsage {
    pub category: Category,
    pub seconds: i64,
}

/// One day worth of aggregated usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayUsage {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    pub seconds: i64,
    pub by_category: Vec<CategoryUsage>,
}
