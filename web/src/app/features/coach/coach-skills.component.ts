import { ChangeDetectionStrategy, Component, effect, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { CoachSkillsResponse, SkillExample } from '../../core/models';

@Component({
  selector: 'app-coach-skills',
  standalone: true,
  imports: [LucideAngularModule, RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './coach-skills.component.html',
})
export class CoachSkillsComponent {
  private readonly api = inject(ApiService);
  readonly lookback = signal<'30d' | '90d' | '180d'>('90d');
  readonly skills = signal<CoachSkillsResponse | null>(null);
  readonly expanded = signal<string | null>(null);
  readonly examplesCache = new Map<string, SkillExample[]>();

  constructor() {
    effect(() => {
      this.api.coachSkills(this.lookback()).subscribe(v => this.skills.set(v));
    });
  }

  setLookback(lb: '30d' | '90d' | '180d') { this.lookback.set(lb); }

  toggle(hash: string) {
    if (this.expanded() === hash) { this.expanded.set(null); return; }
    this.expanded.set(hash);
    if (!this.examplesCache.has(hash)) {
      this.api.coachSkillExamples(hash, 3).subscribe(r => {
        this.examplesCache.set(hash, r.examples);
        // trigger re-render by writing the signal again
        this.expanded.set(hash);
      });
    }
  }

  copyAsSlashCommand(opp: { label: string; command: string | null }, examples?: SkillExample[]) {
    const name = slugifyForFilename(opp.command ?? opp.label);
    const body = examples?.[0]?.text ?? opp.label;
    const snippet = `---\nname: ${name}\ndescription: TODO write a description.\n---\n${body}\n`;
    navigator.clipboard?.writeText(snippet);
  }
}

// Exported for unit testing in L3.
export function slugifyForFilename(input: string): string {
  return input.toLowerCase()
    .replace(/[^\w]+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 40);
}
