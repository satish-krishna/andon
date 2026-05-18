import { HttpClient, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';
import {
  AcceptByLanguage,
  ActiveTimeToday,
  BackfillResult,
  DailySeries,
  DbStats,
  FileHeatmapRow,
  OverviewToday,
  RepoSummary,
  SessionDetail,
  SessionSummary,
  TopRepoEntry,
} from './models';

const BASE = 'http://127.0.0.1:8765';

export interface FilterArgs {
  fromMs?: number;
  toMs?: number;
  models?: string;
  search?: string;
  sort?: string;
  langs?: string;
  repo?: string;
  limit?: number;
}

export interface V2Kpis {
  window: { from: number; to: number; label: string };
  cost: {
    current: number;
    previous: number;
    delta_pct: number | null;
    projected_eom: number;
    day_of_month: number;
    days_in_month: number;
  };
  sessions: { current: number; previous: number; delta_pct: number | null; pace: number };
  tokens: {
    input: { current: number; previous: number; delta_pct: number | null };
    output: { current: number; previous: number; delta_pct: number | null };
    cache_read: { current: number };
    cache_create: { current: number };
  };
}

export interface V2Tape {
  month: string;
  days_in_month: number;
  today_day: number | null;
  current: number[];
  previous: number[];
}

export interface V2CostByModel {
  model: string;
  cost_usd: number;
}

export interface V2AcceptLang {
  language: string;
  accept_rate: number;
  total: number;
}

export interface V2ActiveTime {
  user_seconds: number;
  cli_seconds: number;
  total_seconds: number;
}

export interface V2Session {
  session_id: string;
  started_at: number;
  ended_at: number | null;
  service_version: string | null;
  host_arch: string | null;
  os_type: string | null;
  cost_usd: number;
  tokens_input: number;
  tokens_output: number;
  accepts: number;
  rejects: number;
  aborts: number;
  duration_seconds: number;
  top_model: string | null;
  api_calls: number;
  decisions: number;
  repo_name: string | null;
  repo_root: string | null;
  repo_remote: string | null;
  cwd: string | null;
}

export interface CoverageHint {
  total: number;
  with_repo: number;
}

export interface SessionListResponse {
  sessions: V2Session[];
  coverage: CoverageHint;
}

export interface V2FileRow {
  file_path: string;
  edits: number;
  added: number;
  removed: number;
  last_ts: number;
  accept_rate: number;
  decision_count: number;
  lang: string;
}

export interface V2FilesResponse {
  files: V2FileRow[];
  lang_breakdown: { lang: string; edits: number }[];
  totals: { files: number; edits: number; added: number; removed: number };
}

export interface ForwarderSettings {
  enabled: boolean;
  endpoint: string;
  timeout_ms: number;
  headers: Record<string, string>;
}

export interface AppSettings {
  version: number;
  forwarder: ForwarderSettings;
}

function toParams(args?: FilterArgs): HttpParams {
  let p = new HttpParams();
  if (!args) return p;
  if (args.fromMs !== undefined) p = p.set('from', String(args.fromMs));
  if (args.toMs !== undefined) p = p.set('to', String(args.toMs));
  if (args.models) p = p.set('models', args.models);
  if (args.search) p = p.set('search', args.search);
  if (args.sort) p = p.set('sort', args.sort);
  if (args.langs) p = p.set('langs', args.langs);
  if (args.repo) p = p.set('repo', args.repo);
  if (args.limit !== undefined) p = p.set('limit', String(args.limit));
  return p;
}

@Injectable({ providedIn: 'root' })
export class ApiService {
  private http = inject(HttpClient);

  // ----- v2 (filterable) -----
  kpis(args?: FilterArgs): Observable<V2Kpis> {
    return this.http.get<V2Kpis>(`${BASE}/api/v2/kpis`, { params: toParams(args) });
  }
  tape(month?: string, models?: string): Observable<V2Tape> {
    let p = new HttpParams();
    if (month) p = p.set('month', month);
    if (models) p = p.set('models', models);
    return this.http.get<V2Tape>(`${BASE}/api/v2/tape`, { params: p });
  }
  costByModel(args?: FilterArgs): Observable<V2CostByModel[]> {
    return this.http.get<V2CostByModel[]>(`${BASE}/api/v2/cost-by-model`, { params: toParams(args) });
  }
  acceptByLanguageV2(args?: FilterArgs): Observable<V2AcceptLang[]> {
    return this.http.get<V2AcceptLang[]>(`${BASE}/api/v2/accept-by-language`, { params: toParams(args) });
  }
  activeTime(args?: FilterArgs): Observable<V2ActiveTime> {
    return this.http.get<V2ActiveTime>(`${BASE}/api/v2/active-time`, { params: toParams(args) });
  }
  sessionsV2(args?: FilterArgs): Observable<SessionListResponse> {
    return this.http.get<SessionListResponse>(`${BASE}/api/v2/sessions`, { params: toParams(args) });
  }
  files(args?: FilterArgs): Observable<V2FilesResponse> {
    return this.http.get<V2FilesResponse>(`${BASE}/api/v2/files`, { params: toParams(args) });
  }

  // ----- legacy + non-filterable -----
  overviewToday(): Observable<OverviewToday> {
    return this.http.get<OverviewToday>(`${BASE}/api/overview/today`);
  }
  costByDay(days = 30): Observable<DailySeries> {
    return this.http.get<DailySeries>(`${BASE}/api/overview/cost-by-day?days=${days}`);
  }
  tokensByDay(days = 30): Observable<DailySeries> {
    return this.http.get<DailySeries>(`${BASE}/api/overview/tokens-by-day?days=${days}`);
  }
  acceptByLanguage(): Observable<AcceptByLanguage[]> {
    return this.http.get<AcceptByLanguage[]>(`${BASE}/api/overview/accept-by-language`);
  }
  activeTimeToday(): Observable<ActiveTimeToday> {
    return this.http.get<ActiveTimeToday>(`${BASE}/api/overview/active-time/today`);
  }
  sessions(limit = 100): Observable<SessionSummary[]> {
    return this.http.get<SessionSummary[]>(`${BASE}/api/sessions?limit=${limit}`);
  }
  session(id: string): Observable<SessionDetail> {
    return this.http.get<SessionDetail>(`${BASE}/api/sessions/${encodeURIComponent(id)}`);
  }
  filesHeatmap(days = 30): Observable<FileHeatmapRow[]> {
    return this.http.get<FileHeatmapRow[]>(`${BASE}/api/files/heatmap?days=${days}`);
  }
  stats(): Observable<DbStats> {
    return this.http.get<DbStats>(`${BASE}/api/stats`);
  }
  controlStatus(): Observable<{ paused: boolean }> {
    return this.http.get<{ paused: boolean }>(`${BASE}/api/control/status`);
  }
  getSettings(): Observable<AppSettings> {
    return this.http.get<AppSettings>(`${BASE}/api/settings`);
  }
  saveForwarder(f: ForwarderSettings): Observable<ForwarderSettings> {
    return this.http.put<ForwarderSettings>(`${BASE}/api/settings/forwarder`, f);
  }
  testForwarder(f: ForwarderSettings): Observable<{ ok: boolean; status?: number; error?: string }> {
    return this.http.post<any>(`${BASE}/api/settings/forwarder/test`, f);
  }
  pause(): Observable<{ paused: boolean }> {
    return this.http.post<{ paused: boolean }>(`${BASE}/api/control/pause`, {});
  }
  resume(): Observable<{ paused: boolean }> {
    return this.http.post<{ paused: boolean }>(`${BASE}/api/control/resume`, {});
  }
  openDataFolder(): Observable<{ opened: boolean; path?: string; error?: string }> {
    return this.http.post<{ opened: boolean; path?: string; error?: string }>(
      `${BASE}/api/open-data-folder`,
      {},
    );
  }
  integrationStatus(): Observable<IntegrationStatus> {
    return this.http.get<IntegrationStatus>(`${BASE}/api/integration/status`);
  }
  reapplyIntegration(): Observable<IntegrationStatus> {
    return this.http.post<IntegrationStatus>(`${BASE}/api/integration/reapply`, {});
  }
  unpatchIntegration(): Observable<{ ok: boolean; message?: string; error?: string }> {
    return this.http.post<any>(`${BASE}/api/integration/unpatch`, {});
  }
  restoreIntegrationBackup(): Observable<{ ok: boolean; message?: string; error?: string }> {
    return this.http.post<any>(`${BASE}/api/integration/restore-backup`, {});
  }
  autostartStatus(): Observable<{ enabled: boolean; registered_command: string | null }> {
    return this.http.get<any>(`${BASE}/api/autostart/status`);
  }
  autostartEnable(): Observable<{ ok: boolean; registered_command?: string; error?: string }> {
    return this.http.post<any>(`${BASE}/api/autostart/enable`, {});
  }
  autostartDisable(): Observable<{ ok: boolean; error?: string }> {
    return this.http.post<any>(`${BASE}/api/autostart/disable`, {});
  }
  diagnostics(): Observable<any> {
    return this.http.get<any>(`${BASE}/api/diagnostics`);
  }
  recentEvents(limit = 100, event?: string): Observable<{ events: any[] }> {
    const q = event ? `?limit=${limit}&event=${encodeURIComponent(event)}` : `?limit=${limit}`;
    return this.http.get<{ events: any[] }>(`${BASE}/api/diagnostics/events${q}`);
  }
  exportDiagnostics(): Observable<any> {
    return this.http.get<any>(`${BASE}/api/diagnostics/export`);
  }
  getReport(id: string): Observable<{ exists: boolean; path: string; generated_at: number | null }> {
    return this.http.get<any>(`${BASE}/api/sessions/${encodeURIComponent(id)}/report`);
  }
  generateReport(id: string): Observable<{ exists: boolean; path: string; generated_at: number | null }> {
    return this.http.post<any>(`${BASE}/api/sessions/${encodeURIComponent(id)}/report`, {});
  }
  openReport(id: string): Observable<{ ok: boolean; path?: string; error?: string }> {
    return this.http.post<any>(`${BASE}/api/sessions/${encodeURIComponent(id)}/report/open`, {});
  }
  reportsIndex(): Observable<{ session_ids: string[] }> {
    return this.http.get<any>(`${BASE}/api/sessions/reports/index`);
  }
  listRepos(args: { from?: number; to?: number; limit?: number }): Observable<RepoSummary[]> {
    let p = new HttpParams();
    if (args.from !== undefined) p = p.set('from', String(args.from));
    if (args.to !== undefined) p = p.set('to', String(args.to));
    if (args.limit !== undefined) p = p.set('limit', String(args.limit));
    return this.http.get<RepoSummary[]>(`${BASE}/api/repos`, { params: p });
  }
  topRepos(args: { from: number; to: number; limit?: number }): Observable<TopRepoEntry[]> {
    let p = new HttpParams();
    p = p.set('from', String(args.from));
    p = p.set('to', String(args.to));
    if (args.limit !== undefined) p = p.set('limit', String(args.limit));
    return this.http.get<TopRepoEntry[]>(`${BASE}/api/overview/top-repos`, { params: p });
  }
  backfillRepos(): Observable<BackfillResult> {
    return this.http.post<BackfillResult>(`${BASE}/api/repo/backfill`, {});
  }
}

export type IntegrationStatus =
  | { state: 'already_configured'; settings_path: string }
  | { state: 'patched'; settings_path: string; backup_path: string }
  | { state: 'conflict'; settings_path: string; existing_endpoint: string }
  | { state: 'error'; message: string };
