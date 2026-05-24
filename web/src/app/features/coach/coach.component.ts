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
}
