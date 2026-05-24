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
    #[serde(default)] pub cost_source: Option<String>,
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

#[derive(serde::Serialize, Default, Clone, Copy)]
pub struct LinePair {
    pub added: i64,
    pub removed: i64,
}

#[derive(serde::Serialize, Default)]
pub struct LineSplit {
    pub code: LinePair,
    pub docs: LinePair,
    pub other: LinePair,
}

#[derive(serde::Serialize)]
pub struct SessionTotals {
    pub sessions: i64,
    pub cost_usd: f64,
    pub tokens_input: i64,
    pub tokens_output: i64,
    pub accepts: i64,
    pub rejects: i64,
    pub aborts: i64,
    pub decisions: i64,
    pub duration_seconds: f64,
    /// Sum of all three buckets' `added`.
    pub lines_added: i64,
    /// Sum of all three buckets' `removed`.
    pub lines_removed: i64,
    pub lines: LineSplit,
}

#[derive(serde::Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<serde_json::Value>,
    pub coverage: CoverageHint,
    pub totals: SessionTotals,
}

// ---- JSONL ingest DTOs ----

#[derive(Debug, serde::Serialize)]
pub struct JsonlBackfillResponse {
    pub files_processed: i64,
    pub records_processed: i64,
    pub records_errored: i64,
    pub sessions_added: i64,
    pub tokens_written: i64,
    pub cost_written: i64,
    pub duration_ms: i64,
}

impl From<crate::jsonl::IngestStats> for JsonlBackfillResponse {
    fn from(s: crate::jsonl::IngestStats) -> Self {
        Self {
            files_processed: s.files_processed,
            records_processed: s.records_processed,
            records_errored: s.records_errored,
            sessions_added: s.sessions_added,
            tokens_written: s.tokens_written,
            cost_written: s.cost_written,
            duration_ms: s.duration_ms,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct JsonlErrorEntry {
    pub jsonl_path: String,
    pub line_no: i64,
    pub error_kind: String,
    pub error_msg: String,
    pub cc_version: Option<String>,
    pub ingested_at: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct JsonlIngestRunEntry {
    pub id: i64,
    pub kind: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub files_processed: i64,
    pub records_processed: i64,
    pub records_errored: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct CoverageGap {
    pub session_id: String,
    pub jsonl_calls: i64,
    pub otlp_calls: i64,
}

// ---- Behaviour DTOs ----

#[derive(Debug, serde::Serialize)]
pub struct ModelMixEntry {
    pub model: String,
    pub invocations: i64,
    pub sessions: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct ModelToolCell {
    pub model: String,
    pub tool: String,
    pub count: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct ModelMixResponse {
    pub by_model: Vec<ModelMixEntry>,
    pub by_model_tool: Vec<ModelToolCell>,
}

#[derive(Debug, serde::Serialize)]
pub struct SlashCommandEntry {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct SubAgentEntry {
    pub subagent_type: String,
    pub invocations: i64,
}

// ---- Coach DTOs ----

#[derive(serde::Serialize)]
pub struct CoachScorecardDto {
    pub practices: Vec<crate::coach::score::PracticeRow>,
    pub window: crate::coach::score::WindowDto,
    pub sessions_in_window: i64,
}

#[derive(serde::Deserialize)]
pub struct UpdateCoachRule {
    pub enabled: bool,
}

#[derive(serde::Serialize)]
pub struct CoachRuleDto {
    pub id: String,
    pub practice: String,
    pub severity: Option<String>,
    pub kind: String,
    pub aiec_origin: Option<String>,
    pub description: String,
    pub suggestion: String,
    pub enabled: bool,
    pub reserved: bool,
}

#[derive(serde::Serialize)]
pub struct CoachFindingRow {
    pub id: i64,
    pub rule_id: String,
    pub practice: String,
    pub severity: Option<String>,
    pub session_id: String,
    pub started_at: i64,
    pub detected_at: i64,
    pub repo: Option<String>,
    pub cost_usd: f64,
    pub description: String,
    pub suggestion: String,
    pub payload: serde_json::Value,
}

#[derive(serde::Serialize)]
pub struct CoachFindingsResponse {
    pub items: Vec<CoachFindingRow>,
    pub next_cursor: Option<i64>,
}

// ---- Coach Skills DTOs ----

#[derive(serde::Serialize)]
pub struct SkillOpportunityRow {
    pub norm_hash: String,
    pub label: String,
    pub command: Option<String>,
    pub occurrences: i64,
    pub session_count: i64,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(serde::Serialize)]
pub struct CoachSkillsResponse {
    pub lookback: String,
    pub opportunities: Vec<SkillOpportunityRow>,
}

// ---- Efficiency page DTOs ----

#[derive(Debug, serde::Serialize)]
pub struct CacheTokenTotals {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_create: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct CacheSavings {
    pub net: f64,
    pub gross: f64,
    pub creation_overhead: f64,
}

#[derive(Debug, serde::Serialize)]
pub struct CacheEfficiency {
    pub hit_ratio: f64,
    pub hit_ratio_prev: f64,
    pub tokens: CacheTokenTotals,
    pub savings: CacheSavings,
    pub net_prev: f64,
    pub unpriced_cache_tokens: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct ModelEfficiencyRow {
    /// `opus` | `sonnet` | `haiku` | `other`.
    pub family: String,
    /// `main` (the session's main-agent half) | `subagent` (sidechain).
    pub role: String,
    pub sessions: i64,
    pub total_cost_usd: f64,
    pub cost_per_session: f64,
    pub output_tokens: i64,
    pub cost_per_1k_output: f64,
}
