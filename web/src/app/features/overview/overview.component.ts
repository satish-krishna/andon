import { CommonModule, DatePipe, DecimalPipe, PercentPipe } from '@angular/common';
import { Component, OnInit, computed, effect, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { LucideAngularModule } from 'lucide-angular';
import {
  ApiService,
  V2AcceptLang,
  V2ActiveTime,
  V2CostByModel,
  V2Kpis,
  V2Session,
  V2Tape,
} from '../../core/api.service';
import { ModelMixResponse } from '../../core/models';
import { FilterService } from '../../core/filter.service';
import { FilterBarComponent } from '../../shared/filter-bar.component';
import { TopReposTileComponent } from './top-repos-tile.component';
import { budgetTextClass, budgetBarClass } from './budget-indicator';
import { dateInWindow } from './tape-window';

// Model names come in like "claude-opus-4-7" or "claude-haiku-4-5-20251001".
// Substring match keeps it forward-compatible with new versions.
const MODEL_COLOR_TABLE: [string, string][] = [
  ['opus',   '#facc15'],
  ['sonnet', '#60a5fa'],
  ['haiku',  '#34d399'],
];
const FALLBACK_COLORS = ['#a78bfa', '#f472b6', '#fb923c', '#22d3ee'];

@Component({
  selector: 'app-overview',
  imports: [
    CommonModule,
    DatePipe,
    DecimalPipe,
    PercentPipe,
    RouterLink,
    FilterBarComponent,
    LucideAngularModule,
    TopReposTileComponent,
  ],
  templateUrl: './overview.component.html',
})
export class OverviewComponent implements OnInit {
  filter = inject(FilterService);
  private api = inject(ApiService);

  kpis = signal<V2Kpis | null>(null);
  tape = signal<V2Tape | null>(null);
  costByModel = signal<V2CostByModel[]>([]);
  acceptLang = signal<V2AcceptLang[]>([]);
  activeTime = signal<V2ActiveTime | null>(null);
  recent = signal<V2Session[]>([]);
  modelMix = signal<ModelMixResponse | null>(null);

  // range signals for child tiles
  rangeFrom = computed(() => this.filter.window().fromMs);
  rangeTo   = computed(() => this.filter.window().toMs);

  // Scale bars to the tallest day in the 30-day window.
  tapeMax = computed(() => {
    const t = this.tape();
    if (!t || t.days.length === 0) return 1;
    return Math.max(1, ...t.days.map((d) => d.cost));
  });

  // Range label for the panel title, e.g. "Jun 2 – Jul 1".
  tapeRangeLabel = computed(() => {
    const t = this.tape();
    if (!t || t.days.length === 0) return '';
    const fmt = (iso: string) => {
      const [y, m, d] = iso.split('-').map(Number);
      return new Date(y, m - 1, d).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    };
    return `${fmt(t.days[0].date)} – ${fmt(t.days[t.days.length - 1].date)}`;
  });

  modelColor(m: string | null): string {
    if (!m) return '#7b8794';
    const lower = m.toLowerCase();
    for (const [key, color] of MODEL_COLOR_TABLE) {
      if (lower.includes(key)) return color;
    }
    // hash to a fallback color for stability
    let h = 0;
    for (let i = 0; i < lower.length; i++) h = (h * 31 + lower.charCodeAt(i)) >>> 0;
    return FALLBACK_COLORS[h % FALLBACK_COLORS.length];
  }

  /** Display name: claude-opus-4-7 → "opus 4.7" */
  modelLabel(m: string | null): string {
    if (!m) return '—';
    return m
      .replace(/^claude-/, '')
      .replace(/-(\d+)-(\d+)(?:-\d+)?$/, ' $1.$2');
  }

  /** True when bar `date` ("YYYY-MM-DD") falls inside the active filter window. */
  inWindow(date: string): boolean {
    const w = this.filter.window();
    return dateInWindow(date, w.fromMs, w.toMs);
  }

  /** True when the filter is a single-day custom window on exactly this date. */
  private isSoleSelectedDay(date: string): boolean {
    if (this.filter.range() !== 'custom') return false;
    const w = this.filter.window();
    const from = new Date(w.fromMs);
    const to = new Date(w.toMs);
    if (from.toDateString() !== to.toDateString()) return false;
    const [y, m, d] = date.split('-').map(Number);
    return from.getFullYear() === y && from.getMonth() + 1 === m && from.getDate() === d;
  }

  /**
   * Click tape bar for `date`. Clicking the already-isolated single day toggles
   * back to "This month"; any other day narrows the filter to that day.
   */
  onTapeDayClick(date: string) {
    if (this.isSoleSelectedDay(date)) {
      this.filter.setRange('month');
      return;
    }
    const [y, m, d] = date.split('-').map(Number);
    this.filter.selectDay(new Date(y, m - 1, d));
  }

  /** Tailwind classes for the tape bar on `date`. Today gets a top marker. */
  tapeBarClass(date: string): string {
    const isToday = date === this.tape()?.days.at(-1)?.date;
    if (this.inWindow(date)) {
      return isToday
        ? 'bg-accent border-t border-yellow-200'
        : 'bg-accent group-hover:bg-accent';
    }
    return isToday
      ? 'bg-accent/30 border-t border-yellow-200 group-hover:bg-accent/60'
      : 'bg-accent/30 group-hover:bg-accent/60';
  }

  constructor() {
    // refetch whenever the filter window/models change or Refresh is clicked
    effect(() => {
      this.filter.refreshTick(); // re-run when the Refresh button is clicked
      const w = this.filter.window();
      const models = this.filter.modelsCsv();
      const args = { fromMs: w.fromMs, toMs: w.toMs, models };
      this.api.kpis(args).subscribe((v) => this.kpis.set(v));
      this.api.tape(models).subscribe((v) => this.tape.set(v));
      this.api.costByModel(args).subscribe((v) => this.costByModel.set(v));
      this.api.acceptByLanguageV2(args).subscribe((v) => this.acceptLang.set(v));
      this.api.activeTime(args).subscribe((v) => this.activeTime.set(v));
      this.api.sessionsV2({ ...args, sort: 'time', limit: 6 }).subscribe((v) => this.recent.set(v.sessions));
      this.api.modelMix(args).subscribe((v) => this.modelMix.set(v));
    });
  }

  ngOnInit() {}

  fmtDuration(secs: number): string {
    if (!secs) return '0m';
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
  }

  fmtTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k';
    return String(n);
  }

  fmtDelta(d: number | null): string {
    if (d === null || d === undefined) return '—';
    const sign = d >= 0 ? '▲' : '▾';
    return `${sign} ${Math.abs(d * 100).toFixed(0)}%`;
  }

  deltaClass(d: number | null): string {
    if (d === null || d === undefined) return 'delta-flat';
    return d >= 0 ? 'delta-up' : 'delta-down';
  }

  Math = Math; // template access
  budgetTextClass = budgetTextClass; // template access
  budgetBarClass = budgetBarClass; // template access
  now() { return Date.now(); }
}
