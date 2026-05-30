import { CommonModule, DecimalPipe } from '@angular/common';
import { Component, OnInit, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';

import { RouterLink } from '@angular/router';
import { ApiService, IntegrationStatus } from '../../core/api.service';
import { DbStats, JsonlIngestRun } from '../../core/models';
import { ForwarderCardComponent } from './forwarder-card.component';
import { BudgetCardComponent } from './budget-card.component';
import { AndonMarkComponent } from '../../shared/andon-mark.component';

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [CommonModule, DecimalPipe, RouterLink, LucideAngularModule, ForwarderCardComponent, BudgetCardComponent, AndonMarkComponent],
  templateUrl: './settings.component.html',
})
export class SettingsComponent implements OnInit {
  private api = inject(ApiService);

  stats = signal<DbStats | null>(null);
  paused = signal(false);
  integration = signal<IntegrationStatus | null>(null);
  actionMsg = signal<string>('');
  autostart = signal<{ enabled: boolean; registered_command: string | null } | null>(null);
  backfillResult = signal<{ scanned: number; updated: number } | null>(null);
  backfilling = signal(false);
  version = signal<string | null>(null);
  jsonlBusy = signal(false);
  jsonlToast = signal<{ msg: string; kind: 'ok' | 'err' } | null>(null);
  jsonlLatestRun = signal<JsonlIngestRun | null>(null);

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
    this.api.health().subscribe((h) => this.version.set(h.version));
    this.api.jsonlIngestRuns().subscribe((rs) => this.jsonlLatestRun.set(rs[0] ?? null));
  }

  ingestJsonl() {
    this.jsonlBusy.set(true);
    this.jsonlToast.set(null);
    this.api.ingestJsonl().subscribe({
      next: (s) => {
        const summary =
          `Ingested ${s.records_processed} records from ${s.files_processed} files`;
        const written = s.tokens_written + s.cost_written;
        const tail = written > 0 ? ` · wrote ${written} cost/token rows from JSONL` : '';
        this.jsonlToast.set({
          msg: `${summary}${tail} (${s.records_errored} errors).`,
          kind: 'ok',
        });
        this.jsonlBusy.set(false);
        this.api.jsonlIngestRuns().subscribe((rs) => this.jsonlLatestRun.set(rs[0] ?? null));
      },
      error: (e) => {
        const detail = e?.error?.error ?? e?.message ?? String(e);
        this.jsonlToast.set({ msg: `JSONL ingest failed: ${detail}`, kind: 'err' });
        this.jsonlBusy.set(false);
      },
    });
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

  runBackfill() {
    this.backfilling.set(true);
    this.api.backfillRepos().subscribe({
      next: r => { this.backfillResult.set(r); this.backfilling.set(false); },
      error: () => this.backfilling.set(false),
    });
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
