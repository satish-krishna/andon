import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { ApiService } from '../../core/api.service';

@Component({
  selector: 'app-sweep-card',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="rounded border border-border bg-panel p-3">
      <div class="text-xs text-muted mb-2">Transcript sweep</div>
      <p class="text-[11px] text-muted mb-2">
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
        <span class="text-[11px]" [class.text-ok]="ok()" [class.text-warn]="!ok()">{{ msg() }}</span>
      </div>
    </div>
  `,
})
export class SweepCardComponent implements OnInit {
  private api = inject(ApiService);
  intervalMinutes = signal(5);
  enabled = signal(true);
  dirty = signal(false);
  msg = signal('');
  ok = signal(true);

  ngOnInit() {
    this.api.getSettings().subscribe((s) => {
      this.intervalMinutes.set(s.sweep?.interval_minutes ?? 5);
      this.enabled.set(s.sweep?.enabled ?? true);
      this.dirty.set(false);
    });
  }
  onToggle(e: Event) { this.enabled.set((e.target as HTMLInputElement).checked); this.dirty.set(true); }
  onInterval(e: Event) { this.intervalMinutes.set(Number((e.target as HTMLInputElement).value)); this.dirty.set(true); }
  save() {
    this.api.saveSweep({ interval_minutes: Number(this.intervalMinutes()), enabled: this.enabled() }).subscribe({
      next: () => { this.msg.set('saved'); this.ok.set(true); this.dirty.set(false); },
      error: (e) => { this.msg.set(`error: ${e?.error?.error ?? 'failed'}`); this.ok.set(false); },
    });
  }
}
