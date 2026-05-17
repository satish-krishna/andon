import { CommonModule, DatePipe, DecimalPipe } from '@angular/common';
import { Component, effect, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService, V2Session, V2FileRow } from '../../core/api.service';
import { FilterService } from '../../core/filter.service';
import { FilterBarComponent } from '../../shared/filter-bar.component';
import { SessionDetail } from '../../core/models';

type SortKey = 'time' | 'cost' | 'duration' | 'decisions';

@Component({
  selector: 'app-sessions',
  imports: [CommonModule, DatePipe, DecimalPipe, FormsModule, RouterLink, FilterBarComponent, LucideAngularModule],
  templateUrl: './sessions.component.html',
  standalone: true,
})
export class SessionsComponent {
  filter = inject(FilterService);
  private api = inject(ApiService);

  rows = signal<V2Session[]>([]);
  loaded = signal(false);
  sort = signal<SortKey>('time');
  expanded = signal<string | null>(null);
  detailById = signal<Record<string, SessionDetail | null>>({});

  searchInput = '';

  constructor() {
    effect(() => {
      const w = this.filter.window();
      const models = this.filter.modelsCsv();
      const args = { fromMs: w.fromMs, toMs: w.toMs, models, sort: this.sort(), limit: 200 };
      this.api.sessionsV2({ ...args, search: this.searchInput || undefined }).subscribe((v) => {
        this.rows.set(v);
        this.loaded.set(true);
      });
    });
  }

  onSearch(v: string) {
    this.searchInput = v;
    const w = this.filter.window();
    const models = this.filter.modelsCsv();
    this.api
      .sessionsV2({ fromMs: w.fromMs, toMs: w.toMs, models, sort: this.sort(), limit: 200, search: v || undefined })
      .subscribe((r) => this.rows.set(r));
  }

  setSort(k: SortKey) {
    this.sort.set(k);
  }

  toggleExpand(id: string) {
    const cur = this.expanded();
    this.expanded.set(cur === id ? null : id);
    if (cur !== id && !this.detailById()[id]) {
      this.api.session(id).subscribe((d) => {
        this.detailById.update((m) => ({ ...m, [id]: d }));
      });
    }
  }

  acceptRate(r: V2Session): number {
    const d = r.accepts + r.rejects;
    return d === 0 ? 0 : r.accepts / d;
  }

  fmtDuration(s: number): string {
    if (!s) return '—';
    if (s < 60) return `${Math.round(s)}s`;
    const m = Math.floor(s / 60);
    const r = Math.round(s % 60);
    return `${m}m ${r}s`;
  }

  // group by day for date headers
  groupedRows(): { label: string; rows: V2Session[] }[] {
    const groups: Record<string, V2Session[]> = {};
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const yesterday = today.getTime() - 86_400_000;
    for (const r of this.rows()) {
      const d = new Date(r.started_at);
      d.setHours(0, 0, 0, 0);
      const key =
        d.getTime() === today.getTime() ? 'today' :
        d.getTime() === yesterday ? 'yesterday' :
        d.toLocaleDateString(undefined, { weekday: 'long', month: 'short', day: 'numeric' });
      (groups[key] ||= []).push(r);
    }
    return Object.entries(groups).map(([label, rows]) => ({ label, rows }));
  }

  Math = Math;

  modelLabel(m: string | null): string {
    if (!m) return '—';
    return m.replace(/^claude-/, '').replace(/-(\d+)-(\d+)(?:-\d+)?$/, ' $1.$2');
  }
  modelColor(m: string | null): string {
    if (!m) return '#7b8794';
    const lower = m.toLowerCase();
    if (lower.includes('opus')) return '#facc15';
    if (lower.includes('sonnet')) return '#60a5fa';
    if (lower.includes('haiku')) return '#34d399';
    return '#a78bfa';
  }
}
