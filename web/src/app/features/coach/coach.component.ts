import { ChangeDetectionStrategy, Component, effect, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService } from '../../core/api.service';
import { CoachScorecard, CoachFindingsResponse, CoachSkillsResponse } from '../../core/models';
import { FilterService } from '../../core/filter.service';
import { FilterBarComponent } from '../../shared/filter-bar.component';

@Component({
  selector: 'app-coach',
  standalone: true,
  imports: [RouterLink, LucideAngularModule, FilterBarComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './coach.component.html',
})
export class CoachComponent {
  readonly filter = inject(FilterService);
  private readonly api = inject(ApiService);

  readonly scorecard = signal<CoachScorecard | null>(null);
  readonly findings = signal<CoachFindingsResponse | null>(null);
  readonly skills = signal<CoachSkillsResponse | null>(null);

  constructor() {
    effect(() => {
      this.filter.refreshTick();
      const w = this.filter.window();
      const models = this.filter.modelsCsv();
      const args = { fromMs: w.fromMs, toMs: w.toMs, models };
      this.api.coachScorecard(args).subscribe(v => this.scorecard.set(v));
      this.api.coachFindings({ fromMs: w.fromMs, toMs: w.toMs, limit: 50 }).subscribe(v => this.findings.set(v));
      this.api.coachSkills('90d').subscribe(v => this.skills.set(v));
    });
  }

  practiceLabel(p: string): string {
    const labels: Record<string, string> = {
      prompt: 'Prompt quality',
      hygiene: 'Session hygiene',
      review: 'Code review',
      tool: 'Tool mastery',
      context: 'Context mgmt',
    };
    return labels[p] ?? p;
  }

  continuousLabel(id: string): string {
    return id === 'model-diversity' ? 'Model diversity' : id;
  }

  scoreColor(p: { score: number | null }): 'ok' | 'warn' | 'err' | 'muted' {
    if (p.score == null) return 'muted';
    if (p.score >= 70) return 'ok';
    if (p.score >= 40) return 'warn';
    return 'err';
  }

  trendGlyph(pct: number): string { return pct < 0 ? '▾' : pct > 0 ? '▴' : '—'; }
  trendCls(pct: number): string {
    // Fewer findings is better — invert the colour.
    return pct < 0 ? 'text-ok' : pct > 0 ? 'text-err' : 'text-muted';
  }
  absPct(p: number): number { return Math.abs(p); }
}
