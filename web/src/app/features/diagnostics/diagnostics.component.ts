import { CommonModule, DatePipe } from '@angular/common';
import { Component, OnDestroy, OnInit, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService } from '../../core/api.service';

@Component({
  selector: 'app-diagnostics',
  standalone: true,
  imports: [CommonModule, DatePipe, FormsModule],
  templateUrl: './diagnostics.component.html',
})
export class DiagnosticsComponent implements OnInit, OnDestroy {
  private api = inject(ApiService);

  diag = signal<any>(null);
  events = signal<any[]>([]);
  expandedId = signal<number | null>(null);
  feedFilter = signal<string>('');
  paused = signal(false);

  private pollDiag?: ReturnType<typeof setInterval>;
  private pollFeed?: ReturnType<typeof setInterval>;

  sortedCounters = computed(() => {
    const d = this.diag();
    if (!d?.event_counters) return [];
    return Object.entries(d.event_counters as Record<string, number>)
      .sort((a, b) => (b[1] as number) - (a[1] as number));
  });

  ngOnInit() {
    this.refreshDiag();
    this.refreshFeed();
    this.pollDiag = setInterval(() => this.refreshDiag(), 3000);
    this.pollFeed = setInterval(() => {
      if (!this.paused()) this.refreshFeed();
    }, 2500);
  }

  ngOnDestroy() {
    if (this.pollDiag) clearInterval(this.pollDiag);
    if (this.pollFeed) clearInterval(this.pollFeed);
  }

  refreshDiag() {
    this.api.diagnostics().subscribe((d) => this.diag.set(d));
  }
  refreshFeed() {
    this.api.recentEvents(80, this.feedFilter() || undefined).subscribe((r) => this.events.set(r.events));
  }

  setFilter(e: string) {
    this.feedFilter.set(e === 'all' ? '' : e);
    this.refreshFeed();
  }

  togglePause() {
    this.paused.set(!this.paused());
  }

  toggleExpand(id: number) {
    this.expandedId.set(this.expandedId() === id ? null : id);
  }

  download() {
    this.api.exportDiagnostics().subscribe((report) => {
      const blob = new Blob([JSON.stringify(report, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `andon-diagnostics-${Date.now()}.json`;
      a.click();
      URL.revokeObjectURL(url);
    });
  }

  reapply() {
    this.api.reapplyIntegration().subscribe(() => this.refreshDiag());
  }

  fmtAttrs(attrs: any): string {
    return JSON.stringify(attrs, null, 2);
  }

  bindLabel(s: boolean | null): string {
    if (s === null || s === undefined) return '… pending';
    return s ? '✓ ok' : '✗ failed';
  }
  bindClass(s: boolean | null): string {
    if (s === null || s === undefined) return 'text-muted';
    return s ? 'text-ok' : 'text-err';
  }

  now(): number { return Date.now(); }
}
