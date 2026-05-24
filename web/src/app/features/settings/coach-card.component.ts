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
}
