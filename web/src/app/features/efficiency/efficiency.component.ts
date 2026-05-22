import { DecimalPipe, PercentPipe } from '@angular/common';
import { ChangeDetectionStrategy, Component, effect, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService, V2CacheEfficiency, V2ModelEfficiency } from '../../core/api.service';
import { FilterService } from '../../core/filter.service';
import { FilterBarComponent } from '../../shared/filter-bar.component';

// Family -> bar/dot color. Matches the Overview's MODEL_COLOR_TABLE palette.
const FAMILY_COLORS: Record<string, string> = {
  opus: '#facc15',
  sonnet: '#60a5fa',
  haiku: '#34d399',
  other: '#a78bfa',
};

@Component({
  selector: 'app-efficiency',
  standalone: true,
  imports: [DecimalPipe, PercentPipe, FilterBarComponent, LucideAngularModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './efficiency.component.html',
})
export class EfficiencyComponent {
  readonly filter = inject(FilterService);
  private readonly api = inject(ApiService);

  readonly cache = signal<V2CacheEfficiency | null>(null);
  readonly models = signal<V2ModelEfficiency[]>([]);

  constructor() {
    // Refetch whenever the filter window/models change or Refresh is clicked —
    // the same pattern as OverviewComponent.
    effect(() => {
      this.filter.refreshTick();
      const w = this.filter.window();
      const models = this.filter.modelsCsv();
      const args = { fromMs: w.fromMs, toMs: w.toMs, models };
      this.api.cacheEfficiency(args).subscribe((v) => this.cache.set(v));
      this.api.modelEfficiency(args).subscribe((v) => this.models.set(v));
    });
  }

  familyColor(f: string): string {
    return FAMILY_COLORS[f] ?? FAMILY_COLORS['other'];
  }

  fmtTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k';
    return String(n);
  }

  /** Percentage-point delta between two ratios, e.g. "+7 pts". */
  ptDelta(cur: number, prev: number): string {
    const d = Math.round((cur - prev) * 100);
    return `${d >= 0 ? '+' : ''}${d} pts`;
  }

  /** Signed percent delta of a value vs its previous, e.g. "up 20%". */
  pctDelta(cur: number, prev: number): string {
    if (prev === 0) return '—';
    const d = (cur - prev) / prev;
    return `${d >= 0 ? '▲' : '▾'} ${Math.abs(d * 100).toFixed(0)}%`;
  }
}
