import { CommonModule, DecimalPipe } from '@angular/common';
import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';

import { ApiService } from '../../core/api.service';
import {
  ModelMixResponse,
  SlashCommandEntry,
  SubAgentEntry,
} from '../../core/models';

@Component({
  selector: 'app-behaviour',
  standalone: true,
  imports: [CommonModule, DecimalPipe],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './behaviour.component.html',
})
export class BehaviourComponent {
  private readonly api = inject(ApiService);

  readonly modelMix = signal<ModelMixResponse | null>(null);
  readonly slash = signal<SlashCommandEntry[]>([]);
  readonly subs = signal<SubAgentEntry[]>([]);

  constructor() {
    this.api.modelMix().subscribe((v) => this.modelMix.set(v));
    this.api.slashCommands().subscribe((v) => this.slash.set(v));
    this.api.subagents().subscribe((v) => this.subs.set(v));
  }
}
