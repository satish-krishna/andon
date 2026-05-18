use serde::Serialize;

#[derive(Serialize)]
pub struct OverviewToday {
    pub cost_usd: f64,
    pub sessions: i64,
    pub accept_rate: f64,
    pub tokens_input: i64,
    pub tokens_output: i64,
}

#[derive(Serialize)]
pub struct DailySeries {
    pub days: Vec<String>,
    pub series: Vec<NamedSeries>,
}

#[derive(Serialize)]
pub struct NamedSeries {
    pub name: String,
    pub values: Vec<f64>,
}

#[derive(Serialize)]
pub struct AcceptByLanguage {
    pub language: String,
    pub accept_rate: f64,
    pub total: i64,
}

#[derive(Serialize)]
pub struct ActiveTimeToday {
    pub user_seconds: f64,
    pub cli_seconds: f64,
}

#[derive(Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub cost_usd: f64,
    pub tokens_input: i64,
    pub tokens_output: i64,
    pub accepts: i64,
    pub rejects: i64,
    pub service_version: Option<String>,
    pub host_arch: Option<String>,
    pub os_type: Option<String>,
    #[serde(default)] pub cwd: Option<String>,
    #[serde(default)] pub repo_root: Option<String>,
    #[serde(default)] pub repo_remote: Option<String>,
    #[serde(default)] pub repo_branch: Option<String>,
    #[serde(default)] pub repo_name: Option<String>,
}

#[derive(Serialize)]
pub struct SessionDetail {
    pub session: SessionSummary,
    pub cost_by_model: Vec<KeyValueNum>,
    pub tokens_by_type: Vec<KeyValueNum>,
    pub tool_decisions: Vec<ToolDecisionRow>,
    pub files: Vec<FileRow>,
    pub active_time_seconds: f64,
}

#[derive(Serialize)]
pub struct KeyValueNum {
    pub key: String,
    pub value: f64,
}

#[derive(Serialize)]
pub struct ToolDecisionRow {
    pub timestamp: i64,
    pub tool_name: String,
    pub decision: String,
    pub language: Option<String>,
    pub file_path: Option<String>,
}

#[derive(Serialize)]
pub struct FileRow {
    pub file_path: String,
    pub lines_added: i64,
    pub lines_removed: i64,
}

#[derive(Serialize)]
pub struct FileHeatmapRow {
    pub file_path: String,
    pub edit_count: i64,
    pub accept_rate: f64,
}

#[derive(serde::Deserialize, Debug)]
pub struct SessionContextPayload {
    pub session_id: String,
    pub cwd: Option<String>,
    // Tolerated and ignored:
    #[serde(default)] pub source: Option<String>,
    #[serde(default)] pub transcript_path: Option<String>,
    #[serde(default)] pub hook_event_name: Option<String>,
    #[serde(default)] pub model: Option<String>,
}

#[derive(serde::Serialize)]
pub struct BackfillResult {
    pub scanned: usize,
    pub updated: usize,
}

#[derive(serde::Serialize)]
pub struct RepoSummary {
    /// Grouping key: COALESCE(repo_remote, repo_root, cwd, '—').
    pub key: String,
    /// Display label: repo_name (preferred), else basename of key.
    pub label: String,
    /// `true` when repo_remote is set.
    pub has_remote: bool,
    pub session_count: i64,
    /// Any one repo_root observed for this group (MAX). Used by the Files page
    /// to derive relative paths when the user selects a remote-keyed repo chip.
    pub repo_root: Option<String>,
}

#[derive(serde::Serialize)]
pub struct TopRepoEntry {
    pub key: String,
    pub label: String,
    pub cost_usd: f64,
    pub session_count: i64,
    /// Daily cost series for the period, oldest-first, same length as the period.
    pub spark: Vec<f64>,
}

#[derive(serde::Serialize)]
pub struct CoverageHint {
    pub total: i64,
    pub with_repo: i64,
}

#[derive(serde::Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<serde_json::Value>,
    pub coverage: CoverageHint,
}
