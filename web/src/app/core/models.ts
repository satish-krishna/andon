import { CostSource } from './cost-source';

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
  cost_source: CostSource;
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
  sessions_added: number; tokens_written: number; cost_written: number; duration_ms: number;
}

export interface CoverageGap {
  session_id: string; jsonl_calls: number; otlp_calls: number;
}

// ---- AI Engineering Coach ----

export interface CoachContinuous { id: string; score: number; }
export interface CoachPracticeRow {
  practice: string;
  score: number | null;
  status: 'good' | 'needs-improvement' | 'critical' | 'n/a';
  wow_pct: number;
  mom_pct: number;
  triggered_count: number;
  continuous: CoachContinuous[];
}
export interface CoachScorecard {
  practices: CoachPracticeRow[];
  window: { from: number; to: number };
  sessions_in_window: number;
}
export interface CoachFinding {
  id: number;
  rule_id: string;
  practice: string;
  severity: string | null;
  session_id: string;
  started_at: number;
  detected_at: number;
  repo: string | null;
  cost_usd: number;
  description: string;
  suggestion: string;
  payload: Record<string, unknown>;
}
export interface CoachFindingsResponse { items: CoachFinding[]; next_cursor: number | null; }
export interface CoachRule {
  id: string;
  practice: string;
  severity: string | null;
  kind: 'binary' | 'continuous';
  aiec_origin: string | null;
  description: string;
  suggestion: string;
  enabled: boolean;
  reserved: boolean;
}
export interface SkillOpportunity {
  norm_hash: string; label: string; command: string | null;
  occurrences: number; session_count: number;
  first_seen: number; last_seen: number;
}
export interface CoachSkillsResponse { lookback: string; opportunities: SkillOpportunity[]; }
export interface SkillExample { session_id: string; turn_index: number; ts: number; text: string; }
export interface CoachSettings {
  skill_min_occurrences: number;
  skill_min_sessions: number;
  planning_commands: string[];
  planning_keywords: string[];
  constraint_keywords: string[];
}
