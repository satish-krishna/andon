import { CommonModule, DecimalPipe } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService } from '../../core/api.service';
import {
  ModelMixResponse,
  SlashCommandEntry,
  SubAgentEntry,
} from '../../core/models';

@Component({
  selector: 'app-behaviour',
  standalone: true,
  imports: [CommonModule, DecimalPipe, LucideAngularModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './behaviour.component.html',
})
export class BehaviourComponent {
  private readonly api = inject(ApiService);

  readonly modelMix = signal<ModelMixResponse | null>(null);
  readonly slash = signal<SlashCommandEntry[]>([]);
  readonly subs = signal<SubAgentEntry[]>([]);

  // Bar denominators. `by_model` is sorted by invocations, not sessions, so
  // each bar needs its own max. `Math.max(1, ...)` guards against an empty
  // array (-> 1) and division by zero.
  readonly invocationsMax = computed(() =>
    Math.max(1, ...(this.modelMix()?.by_model ?? []).map((m) => m.invocations)),
  );
  readonly sessionsMax = computed(() =>
    Math.max(1, ...(this.modelMix()?.by_model ?? []).map((m) => m.sessions)),
  );

  constructor() {
    this.api.modelMix().subscribe((v) => this.modelMix.set(v));
    this.api.slashCommands().subscribe((v) => this.slash.set(v));
    this.api.subagents().subscribe((v) => this.subs.set(v));
  }
}
