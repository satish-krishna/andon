import { CommonModule, DecimalPipe } from '@angular/common';
import { Component, OnInit, signal } from '@angular/core';

import { ApiService } from '../../core/api.service';
import { DbStats } from '../../core/models';
import { PanelComponent } from '../../shared/panel.component';

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [CommonModule, DecimalPipe, PanelComponent],
  template: `
    <div class="p-6 flex flex-col gap-4 max-w-3xl">
      <h1 class="text-xl font-semibold">Settings</h1>

      <app-panel title="Ingestion">
        <div class="flex items-center gap-3">
          <span class="text-sm">Status:</span>
          <span class="font-mono"
                [class.text-green-400]="!paused()"
                [class.text-amber-400]="paused()">
            {{ paused() ? 'paused' : 'active' }}
          </span>
          <button class="ml-auto px-3 py-1 rounded bg-border hover:bg-border/70 text-sm"
                  (click)="toggle()">
            {{ paused() ? 'Resume' : 'Pause' }}
          </button>
        </div>
      </app-panel>

      <app-panel title="Database">
        <div class="flex items-center gap-2">
          <div class="text-xs font-mono text-muted break-all flex-1">{{ stats()?.db_path }}</div>
          <button class="px-3 py-1 rounded bg-border hover:bg-border/70 text-xs"
                  (click)="openFolder()">Open data folder</button>
        </div>
        @if (stats(); as s) {
          <table class="text-sm font-mono mt-2">
            <tbody>
              @for (entry of tableRows(s); track entry[0]) {
                <tr class="border-b border-border/30">
                  <td class="pr-6 py-1 text-muted">{{ entry[0] }}</td>
                  <td class="text-right py-1">{{ entry[1] | number }}</td>
                </tr>
              }
            </tbody>
          </table>
        }
      </app-panel>

      <app-panel title="Claude Code setup">
        <p class="text-sm text-muted">
          Add the following to <code class="text-accent">~/.claude/settings.json</code> to enable telemetry export:
        </p>
        <pre class="bg-bg p-3 rounded text-xs font-mono overflow-auto"><code>{{ settingsSnippet }}</code></pre>
      </app-panel>

      <app-panel title="About">
        <div class="text-sm">andon — local Claude Code dashboard.</div>
        <div class="text-xs text-muted mt-1">
          Listens on 127.0.0.1:4317 (gRPC), 4318 (HTTP), 8765 (UI API).
          All data stays on this machine.
        </div>
      </app-panel>
    </div>
  `,
})
export class SettingsComponent implements OnInit {
  stats = signal<DbStats | null>(null);
  paused = signal(false);

  settingsSnippet = `{
  "env": {
    "CLAUDE_CODE_ENABLE_TELEMETRY": "1",
    "OTEL_METRICS_EXPORTER": "otlp",
    "OTEL_LOGS_EXPORTER": "otlp",
    "OTEL_EXPORTER_OTLP_PROTOCOL": "grpc",
    "OTEL_EXPORTER_OTLP_ENDPOINT": "http://localhost:4317"
  }
}`;

  constructor(private api: ApiService) {}

  ngOnInit() {
    this.api.stats().subscribe((s) => this.stats.set(s));
    this.api.controlStatus().subscribe((c) => this.paused.set(c.paused));
  }

  toggle() {
    const next = !this.paused();
    (next ? this.api.pause() : this.api.resume()).subscribe((r) => this.paused.set(r.paused));
  }

  openFolder() {
    this.api.openDataFolder().subscribe();
  }

  tableRows(s: DbStats): [string, number][] {
    return Object.entries(s.tables);
  }
}
