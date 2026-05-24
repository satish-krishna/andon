import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { ApiService } from '../../core/api.service';
import { CoachSettings, CoachRule } from '../../core/models';

@Component({
  selector: 'app-coach-card',
  standalone: true,
  imports: [LucideAngularModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './coach-card.component.html',
})
export class CoachCardComponent {
  private readonly api = inject(ApiService);
  readonly settings = signal<CoachSettings | null>(null);
  readonly rules = signal<CoachRule[]>([]);

  constructor() {
    this.api.coachSettings().subscribe(v => this.settings.set(v));
    this.api.coachRules().subscribe(v => this.rules.set(v));
  }

  updateThreshold(which: 'occ' | 'sess', value: string) {
    const v = Math.max(1, Number(value) | 0);
    const cur = this.settings();
    if (!cur) return;
    const next: CoachSettings = { ...cur,
      skill_min_occurrences: which === 'occ' ? v : cur.skill_min_occurrences,
      skill_min_sessions:    which === 'sess' ? v : cur.skill_min_sessions };
    this.api.saveCoachSettings(next).subscribe(saved => this.settings.set(saved));
  }
}
