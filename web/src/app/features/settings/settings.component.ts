import { CommonModule, DecimalPipe } from '@angular/common';
import { Component, OnInit, signal } from '@angular/core';

import { ApiService, IntegrationStatus } from '../../core/api.service';
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

      <app-panel title="Claude Code integration">
        @if (integration(); as i) {
          @switch (i.state) {
            @case ('already_configured') {
              <div class="text-sm flex items-center gap-2">
                <span class="text-green-400">✓ configured</span>
                <span class="text-muted text-xs font-mono">{{ i.settings_path }}</span>
              </div>
              <div class="text-xs text-muted">OTel env vars are present in your Claude Code settings.</div>
            }
            @case ('patched') {
              <div class="text-sm">
                <span class="text-accent">↻ patched</span>
                <span class="text-muted text-xs font-mono ml-2">{{ i.settings_path }}</span>
              </div>
              <div class="text-xs text-muted">
                Andon added the required OTel env vars. Backup at
                <code class="text-accent">{{ i.backup_path }}</code>.
                Restart any open Claude Code sessions.
              </div>
            }
            @case ('conflict') {
              <div class="text-sm text-amber-400">⚠ conflict — manual review needed</div>
              <div class="text-xs text-muted">
                Your settings.json already targets a different OTLP endpoint:
                <code class="text-accent">{{ i.existing_endpoint }}</code>.
                Andon won't overwrite it. Either point it at
                <code class="text-accent">http://localhost:4317</code>
                or remove the variable to let andon take over.
              </div>
            }
            @case ('error') {
              <div class="text-sm text-red-400">✗ error</div>
              <div class="text-xs text-muted font-mono">{{ i.message }}</div>
            }
          }
          <div class="flex items-center gap-2 mt-2">
            <button class="px-3 py-1 rounded bg-border hover:bg-border/70 text-xs"
                    (click)="reapply()">Re-apply</button>
          </div>
        } @else {
          <div class="text-xs text-muted">Checking…</div>
        }
      </app-panel>

      <app-panel title="Manual setup (reference)">
        <p class="text-sm text-muted">
          If you'd rather configure it yourself, add the following to
          <code class="text-accent">~/.claude/settings.json</code>:
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
  integration = signal<IntegrationStatus | null>(null);

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
    this.api.integrationStatus().subscribe((i) => this.integration.set(i));
  }

  reapply() {
    this.api.reapplyIntegration().subscribe((i) => this.integration.set(i));
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
