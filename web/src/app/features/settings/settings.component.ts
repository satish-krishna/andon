import { CommonModule, DecimalPipe } from '@angular/common';
import { Component, OnInit, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService, IntegrationStatus } from '../../core/api.service';
import { DbStats } from '../../core/models';
import { ForwarderCardComponent } from './forwarder-card.component';

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [CommonModule, DecimalPipe, LucideAngularModule, ForwarderCardComponent],
  templateUrl: './settings.component.html',
})
export class SettingsComponent implements OnInit {
  private api = inject(ApiService);

  stats = signal<DbStats | null>(null);
  paused = signal(false);
  integration = signal<IntegrationStatus | null>(null);
  actionMsg = signal<string>('');
  autostart = signal<{ enabled: boolean; registered_command: string | null } | null>(null);

  settingsSnippet = `{
  "env": {
    "CLAUDE_CODE_ENABLE_TELEMETRY": "1",
    "OTEL_METRICS_EXPORTER": "otlp",
    "OTEL_LOGS_EXPORTER": "otlp",
    "OTEL_EXPORTER_OTLP_PROTOCOL": "grpc",
    "OTEL_EXPORTER_OTLP_ENDPOINT": "http://localhost:4317"
  }
}`;

  ngOnInit() {
    this.refresh();
  }
  refresh() {
    this.api.stats().subscribe((s) => this.stats.set(s));
    this.api.controlStatus().subscribe((c) => this.paused.set(c.paused));
    this.api.integrationStatus().subscribe((i) => this.integration.set(i));
    this.api.autostartStatus().subscribe((a) => this.autostart.set(a));
  }

  toggleAutostart() {
    const next = !(this.autostart()?.enabled ?? false);
    const call$ = next ? this.api.autostartEnable() : this.api.autostartDisable();
    call$.subscribe((r) => {
      this.flash(r.ok ? (next ? 'autostart enabled' : 'autostart disabled') : `error: ${r.error}`);
      this.api.autostartStatus().subscribe((a) => this.autostart.set(a));
    });
  }

  togglePause() {
    const next = !this.paused();
    (next ? this.api.pause() : this.api.resume()).subscribe((r) => this.paused.set(r.paused));
  }

  reapplyIntegration() {
    this.api.reapplyIntegration().subscribe((i) => {
      this.integration.set(i);
      this.flash('integration re-applied');
    });
  }

  unpatch() {
    if (!confirm('Remove andon env vars from settings.json? Claude Code will stop sending telemetry to andon until you re-apply.')) return;
    this.api.unpatchIntegration().subscribe((r) => {
      this.flash(r.ok ? (r.message || 'unpatched') : `error: ${r.error}`);
      this.refresh();
    });
  }

  restoreBackup() {
    if (!confirm('Restore the original settings.json from the andon-backup file? Current contents will be overwritten.')) return;
    this.api.restoreIntegrationBackup().subscribe((r) => {
      this.flash(r.ok ? (r.message || 'restored') : `error: ${r.error}`);
      this.refresh();
    });
  }

  openFolder() {
    this.api.openDataFolder().subscribe();
  }

  copySnippet() {
    navigator.clipboard?.writeText(this.settingsSnippet);
    this.flash('copied');
  }

  flash(msg: string) {
    this.actionMsg.set(msg);
    setTimeout(() => this.actionMsg.set(''), 3000);
  }

  tableRows(s: DbStats): [string, number][] {
    return Object.entries(s.tables);
  }

  integrationBadgeClass(i: IntegrationStatus | null): string {
    if (!i) return 'text-muted';
    switch (i.state) {
      case 'already_configured':
      case 'patched': return 'text-ok';
      case 'conflict': return 'text-warn';
      case 'error': return 'text-err';
    }
  }
  integrationBadgeText(i: IntegrationStatus | null): string {
    if (!i) return '…';
    switch (i.state) {
      case 'already_configured': return '✓ configured';
      case 'patched': return '↻ patched';
      case 'conflict': return '⚠ conflict';
      case 'error': return '✗ error';
    }
  }
}
