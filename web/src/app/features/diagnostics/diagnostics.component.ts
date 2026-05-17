import { CommonModule, DatePipe, DecimalPipe } from '@angular/common';
import { Component, OnDestroy, OnInit, signal } from '@angular/core';

import { ApiService } from '../../core/api.service';
import { PanelComponent } from '../../shared/panel.component';
import { EmptyComponent } from '../../shared/empty.component';

@Component({
    selector: 'app-diagnostics',
    imports: [CommonModule, DatePipe, DecimalPipe, PanelComponent, EmptyComponent],
    template: `
    <div class="p-6 flex flex-col gap-4">
      <div class="flex items-center gap-3">
        <h1 class="text-xl font-semibold">Diagnostics</h1>
        <button class="ml-auto px-3 py-1 rounded-sm bg-border hover:bg-border/70 text-xs"
                (click)="refresh()">Refresh</button>
        <button class="px-3 py-1 rounded-sm bg-border hover:bg-border/70 text-xs"
                (click)="download()">Download report</button>
      </div>

      @if (diag(); as d) {
        <app-panel title="Health">
          @switch (d.health.state) {
            @case ('healthy') {
              <div class="text-sm text-green-400">✓ healthy — receiving telemetry</div>
            }
            @case ('starting') {
              <div class="text-sm text-amber-400">… starting up, give it a few seconds</div>
            }
            @case ('no_data') {
              <div class="text-sm text-amber-400">⚠ no data received yet</div>
              <div class="text-xs text-muted mt-1">{{ d.health.reason }}</div>
            }
            @case ('bind_failed') {
              <div class="text-sm text-red-400">✗ port bind failed</div>
              <div class="text-xs text-muted mt-1">
                Ports that failed: {{ d.health.ports.join(', ') }}.
                Another process likely owns them — close other OTel collectors or andon instances.
              </div>
            }
          }
        </app-panel>

        <div class="grid grid-cols-4 gap-4">
          <app-panel title="Records received">
            <div class="text-3xl font-mono">{{ d.total_records | number }}</div>
            <div class="text-xs text-muted">{{ d.total_payloads | number }} payloads</div>
          </app-panel>
          <app-panel title="Last event">
            @if (d.seconds_since_last_event !== null) {
              <div class="text-3xl font-mono">{{ d.seconds_since_last_event }}s ago</div>
              <div class="text-xs text-muted">{{ d.last_event_ms | date : 'mediumTime' }}</div>
            } @else {
              <div class="text-3xl font-mono text-muted">—</div>
              <div class="text-xs text-muted">no events yet</div>
            }
          </app-panel>
          <app-panel title="Uptime">
            <div class="text-3xl font-mono">{{ d.uptime_seconds | number }}s</div>
            <div class="text-xs text-muted">started {{ d.start_ms | date : 'mediumTime' }}</div>
          </app-panel>
          <app-panel title="Last session">
            <div class="text-sm font-mono break-all">
              {{ d.last_session_id?.slice(0, 16) || '—' }}{{ d.last_session_id ? '…' : '' }}
            </div>
          </app-panel>
        </div>

        <div class="grid grid-cols-2 gap-4">
          <app-panel title="Bind status">
            <div class="text-sm space-y-1">
              <div>4317 gRPC (OTLP)  — {{ formatBind(d.binds.grpc_4317) }}</div>
              <div>4318 HTTP (OTLP)  — {{ formatBind(d.binds.http_4318) }}</div>
              <div>8765 API (UI)    — {{ formatBind(d.binds.api_8765) }}</div>
            </div>
            @if (d.last_error) {
              <div class="mt-2 text-xs text-red-400 font-mono">{{ d.last_error }}</div>
            }
          </app-panel>
          <app-panel title="Transport counters">
            @if (objectEntries(d.transport_counters).length === 0) {
              <app-empty message="No payloads received yet." />
            } @else {
              <ul class="text-sm font-mono">
                @for (e of objectEntries(d.transport_counters); track e[0]) {
                  <li class="flex justify-between border-b border-border/30 py-1">
                    <span>{{ e[0] }}</span>
                    <span>{{ e[1] | number }}</span>
                  </li>
                }
              </ul>
            }
          </app-panel>
        </div>

        <app-panel title="Event counters">
          @if (objectEntries(d.event_counters).length === 0) {
            <app-empty message="No events received yet." />
          } @else {
            <table class="w-full text-sm font-mono">
              <thead>
                <tr class="text-left text-xs uppercase text-muted">
                  <th class="px-2 py-1">Event</th>
                  <th class="px-2 py-1 text-right">Count</th>
                  <th class="px-2 py-1"></th>
                </tr>
              </thead>
              <tbody>
                @for (e of sortedEvents(d.event_counters); track e[0]) {
                  <tr class="border-t border-border/30">
                    <td class="px-2 py-1">{{ e[0] }}</td>
                    <td class="px-2 py-1 text-right">{{ e[1] | number }}</td>
                    <td class="px-2 py-1 text-right">
                      <button class="text-accent hover:underline text-xs"
                              (click)="loadEvent(e[0])">view</button>
                    </td>
                  </tr>
                }
              </tbody>
            </table>
          }
        </app-panel>

        <app-panel [title]="eventsTitle()">
          @if (events().length === 0) {
            <app-empty message="No events. Click 'view' next to an event name above to load samples." />
          } @else {
            <div class="text-xs font-mono max-h-[500px] overflow-auto space-y-2">
              @for (e of events(); track e.id) {
                <details class="bg-bg rounded-sm p-2">
                  <summary class="cursor-pointer flex gap-3">
                    <span class="text-muted w-24">{{ e.timestamp | date : 'HH:mm:ss.SSS' }}</span>
                    <span class="text-accent w-48">{{ e.event_name }}</span>
                    <span class="text-muted truncate">{{ e.session_id?.slice(0, 8) || '—' }}…</span>
                    <span class="ml-auto text-muted text-[10px]">{{ e.transport }}</span>
                  </summary>
                  <pre class="mt-2 text-[11px] whitespace-pre-wrap break-all">{{ formatAttrs(e.attributes) }}</pre>
                </details>
              }
            </div>
          }
        </app-panel>
      } @else {
        <app-empty message="Loading…" />
      }
    </div>
  `
})
export class DiagnosticsComponent implements OnInit, OnDestroy {
  diag = signal<any>(null);
  events = signal<any[]>([]);
  selectedEvent = signal<string>('all');
  private poll?: ReturnType<typeof setInterval>;

  constructor(private api: ApiService) {}

  ngOnInit() {
    this.refresh();
    this.loadEvent();
    this.poll = setInterval(() => this.refresh(), 3000);
  }

  ngOnDestroy() {
    if (this.poll) clearInterval(this.poll);
  }

  refresh() {
    this.api.diagnostics().subscribe((d) => this.diag.set(d));
  }

  loadEvent(name?: string) {
    this.selectedEvent.set(name ?? 'all');
    this.api.recentEvents(50, name).subscribe((r) => this.events.set(r.events));
  }

  eventsTitle(): string {
    const s = this.selectedEvent();
    return s === 'all' ? 'Recent events (all)' : `Recent events: ${s}`;
  }

  formatBind(s: boolean | null | undefined): string {
    if (s === null || s === undefined) return '… pending';
    return s ? '✓ ok' : '✗ failed';
  }

  objectEntries(o: any): [string, number][] {
    return o ? Object.entries(o) : [];
  }

  sortedEvents(o: any): [string, number][] {
    return this.objectEntries(o).sort((a, b) => b[1] - a[1]);
  }

  formatAttrs(attrs: any): string {
    return JSON.stringify(attrs, null, 2);
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
}
