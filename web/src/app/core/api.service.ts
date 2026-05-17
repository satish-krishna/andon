import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';
import {
  AcceptByLanguage,
  ActiveTimeToday,
  DailySeries,
  DbStats,
  FileHeatmapRow,
  OverviewToday,
  SessionDetail,
  SessionSummary,
} from './models';

const BASE = 'http://127.0.0.1:8765';

@Injectable({ providedIn: 'root' })
export class ApiService {
  private http = inject(HttpClient);

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
  pause(): Observable<{ paused: boolean }> {
    return this.http.post<{ paused: boolean }>(`${BASE}/api/control/pause`, {});
  }
  resume(): Observable<{ paused: boolean }> {
    return this.http.post<{ paused: boolean }>(`${BASE}/api/control/resume`, {});
  }
}
