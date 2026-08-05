import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { ApiService } from '../../core/api.service';

@Component({
  selector: 'app-sweep-card',
  standalone: true,
  imports: [LucideAngularModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
  <section class="panel" id="sweep">
    <div class="panel-title">
      <span class="flex items-center gap-1.5">
        <lucide-icon name="refresh-cw" class="w-3.5 h-3.5"></lucide-icon>Transcript sweep
      </span>
    </div>
    <div class="panel-body">
      <p class="text-[12px] text-muted mb-3">
        When live telemetry is not received, ingest Claude Code transcripts from disk automatically.
      </p>
      <label class="flex items-center gap-2 mb-2 text-[12px]">
        <input type="checkbox" [checked]="enabled()" (change)="onToggle($event)" />
        Enabled
      </label>
      <div class="flex items-center gap-2">
        <span class="text-[12px] text-muted">Every</span>
        <input class="w-24 bg-bg border border-border rounded px-2 py-1 text-[12px] font-mono"
               type="number" min="1" max="1440" step="1"
               [value]="intervalMinutes()" (input)="onInterval($event)" [disabled]="!enabled()" />
        <span class="text-[12px] text-muted">minutes</span>
        <button class="filter-chip" [disabled]="!dirty()" (click)="save()">save</button>
        @if (msg()) {
          <span class="text-[11px] font-mono" [class.text-accent]="ok()" [class.text-err]="!ok()">{{ msg() }}</span>
        }
      </div>
    </div>
  </section>
  `,
})
export class SweepCardComponent implements OnInit {
  private api = inject(ApiService);
  intervalMinutes = signal(5);
  enabled = signal(true);
  dirty = signal(false);
  msg = signal('');
  ok = signal(false);

  ngOnInit() {
    this.api.getSettings().subscribe((s) => {
      this.intervalMinutes.set(s.sweep?.interval_minutes ?? 5);
      this.enabled.set(s.sweep?.enabled ?? true);
      this.dirty.set(false);
    });
  }
  onToggle(e: Event) { this.enabled.set((e.target as HTMLInputElement).checked); this.dirty.set(true); this.msg.set(''); }
  onInterval(e: Event) { this.intervalMinutes.set(Number((e.target as HTMLInputElement).value)); this.dirty.set(true); this.msg.set(''); }
  save() {
    this.api.saveSweep({ interval_minutes: Number(this.intervalMinutes()), enabled: this.enabled() }).subscribe({
      next: () => { this.flash('saved', true); this.dirty.set(false); },
      error: (e) => this.flash(`error: ${e?.error?.error ?? 'failed'}`, false),
    });
  }

  private flash(text: string, ok: boolean) {
    this.msg.set(text);
    this.ok.set(ok);
    setTimeout(() => {
      this.msg.set('');
      this.ok.set(false);
    }, 4000);
  }
}
