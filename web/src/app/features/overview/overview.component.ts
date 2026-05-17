import { CommonModule, DatePipe, DecimalPipe, PercentPipe } from '@angular/common';
import { Component, OnInit, computed, effect, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import {
  ApiService,
  V2AcceptLang,
  V2ActiveTime,
  V2CostByModel,
  V2Kpis,
  V2Session,
  V2Tape,
} from '../../core/api.service';
import { FilterService } from '../../core/filter.service';
import { FilterBarComponent } from '../../shared/filter-bar.component';

const MODEL_COLORS: Record<string, string> = {
  opus: '#facc15',
  sonnet: '#60a5fa',
  haiku: '#34d399',
};

@Component({
  selector: 'app-overview',
  imports: [
    CommonModule,
    DatePipe,
    DecimalPipe,
    PercentPipe,
    RouterLink,
    FilterBarComponent,
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

  // tape max for scaling
  tapeMax = computed(() => {
    const t = this.tape();
    if (!t) return 1;
    return Math.max(1, ...t.current, ...t.previous);
  });

  modelColor(m: string): string {
    return MODEL_COLORS[m] ?? '#7b8794';
  }

  constructor() {
    // refetch whenever the filter window or models change
    effect(() => {
      const w = this.filter.window();
      const models = this.filter.modelsCsv();
      const args = { fromMs: w.fromMs, toMs: w.toMs, models };
      this.api.kpis(args).subscribe((v) => this.kpis.set(v));
      this.api.tape(undefined, models).subscribe((v) => this.tape.set(v));
      this.api.costByModel(args).subscribe((v) => this.costByModel.set(v));
      this.api.acceptByLanguageV2(args).subscribe((v) => this.acceptLang.set(v));
      this.api.activeTime(args).subscribe((v) => this.activeTime.set(v));
      this.api.sessionsV2({ ...args, sort: 'time', limit: 6 }).subscribe((v) => this.recent.set(v));
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

  tapeMax_ = this.tapeMax; // expose to template
  Math = Math; // template access
  now() { return Date.now(); }
}
