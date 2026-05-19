export interface OverviewToday {
  cost_usd: number;
  sessions: number;
  accept_rate: number;
  tokens_input: number;
  tokens_output: number;
}

export interface NamedSeries {
  name: string;
  values: number[];
}

export interface DailySeries {
  days: string[];
  series: NamedSeries[];
}

export interface AcceptByLanguage {
  language: string;
  accept_rate: number;
  total: number;
}

export interface ActiveTimeToday {
  user_seconds: number;
  cli_seconds: number;
}

export interface SessionSummary {
  session_id: string;
  started_at: number;
  ended_at: number | null;
  cost_usd: number;
  tokens_input: number;
  tokens_output: number;
  accepts: number;
  rejects: number;
  service_version: string | null;
  host_arch: string | null;
  os_type: string | null;
  cwd?: string | null;
  repo_root?: string | null;
  repo_remote?: string | null;
  repo_branch?: string | null;
  repo_name?: string | null;
}

export interface KeyValueNum {
  key: string;
  value: number;
}

export interface ToolDecisionRow {
  timestamp: number;
  tool_name: string;
  decision: string;
  language: string | null;
  file_path: string | null;
}

export interface FileRow {
  file_path: string;
  lines_added: number;
  lines_removed: number;
}

export interface SessionDetail {
  session: SessionSummary;
  cost_by_model: KeyValueNum[];
  tokens_by_type: KeyValueNum[];
  tool_decisions: ToolDecisionRow[];
  files: FileRow[];
  active_time_seconds: number;
}

export interface FileHeatmapRow {
  file_path: string;
  edit_count: number;
  accept_rate: number;
}

export interface DbStats {
  db_path: string;
  tables: Record<string, number>;
}

export interface RepoSummary {
  key: string;
  label: string;
  has_remote: boolean;
  session_count: number;
  repo_root?: string | null;
}

export interface TopRepoEntry {
  key: string;
  label: string;
  cost_usd: number;
  session_count: number;
  spark: number[];
}

export interface BackfillResult {
  scanned: number;
  updated: number;
}

// ---- Behaviour + JSONL ingest ----

export interface ModelMixEntry { model: string; invocations: number; sessions: number; }
export interface ModelToolCell { model: string; tool: string; count: number; }
export interface ModelMixResponse { by_model: ModelMixEntry[]; by_model_tool: ModelToolCell[]; }
export interface SlashCommandEntry { name: string; count: number; }
export interface SubAgentEntry { subagent_type: string; invocations: number; }
export interface JsonlErrorEntry {
  jsonl_path: string; line_no: number; error_kind: string;
  error_msg: string; cc_version: string | null; ingested_at: number;
}
export interface JsonlIngestRun {
  id: number; kind: string; started_at: number; ended_at: number | null;
  files_processed: number; records_processed: number; records_errored: number;
}
export interface JsonlBackfillResponse {
  files_processed: number; records_processed: number; records_errored: number;
  sessions_added: number; tokens_filled: number; cost_filled: number; duration_ms: number;
}
