import { CommonModule, DatePipe, DecimalPipe, PercentPipe } from '@angular/common';
import { Component, OnInit, computed, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService } from '../../core/api.service';
import { SessionSummary } from '../../core/models';
import { PanelComponent } from '../../shared/panel.component';
import { EmptyComponent } from '../../shared/empty.component';

@Component({
    selector: 'app-sessions',
    imports: [
        CommonModule,
        DatePipe,
        DecimalPipe,
        PercentPipe,
        RouterLink,
        PanelComponent,
        EmptyComponent,
    ],
    template: `
    <div class="p-6 flex flex-col gap-4">
      <h1 class="text-xl font-semibold">Sessions</h1>

      <app-panel>
        @if (loaded() && rows().length === 0) {
          <app-empty />
        } @else {
          <table class="w-full text-sm">
            <thead>
              <tr class="text-left text-xs uppercase text-muted">
                <th class="px-2 py-2">Started</th>
                <th class="px-2 py-2">Session</th>
                <th class="px-2 py-2 text-right">Cost</th>
                <th class="px-2 py-2 text-right">In</th>
                <th class="px-2 py-2 text-right">Out</th>
                <th class="px-2 py-2 text-right">Accept</th>
                <th class="px-2 py-2">Version</th>
              </tr>
            </thead>
            <tbody>
              @for (r of rows(); track r.session_id) {
                <tr class="border-t border-border hover:bg-border/30 cursor-pointer">
                  <td class="px-2 py-2 font-mono text-xs">{{ r.started_at | date : 'MMM d, HH:mm' }}</td>
                  <td class="px-2 py-2 font-mono text-xs">
                    <a [routerLink]="['/sessions', r.session_id]" class="text-accent hover:underline">
                      {{ r.session_id.slice(0, 8) }}…
                    </a>
                  </td>
                  <td class="px-2 py-2 text-right font-mono">$ {{ r.cost_usd | number : '1.4-4' }}</td>
                  <td class="px-2 py-2 text-right font-mono">{{ r.tokens_input | number }}</td>
                  <td class="px-2 py-2 text-right font-mono">{{ r.tokens_output | number }}</td>
                  <td class="px-2 py-2 text-right font-mono">{{ acceptRate(r) | percent : '1.0-1' }}</td>
                  <td class="px-2 py-2 text-xs text-muted">{{ r.service_version || '—' }}</td>
                </tr>
              }
            </tbody>
          </table>
        }
      </app-panel>
    </div>
  `
})
export class SessionsComponent implements OnInit {
  rows = signal<SessionSummary[]>([]);
  loaded = signal(false);

  constructor(private api: ApiService) {}

  ngOnInit() {
    this.api.sessions(200).subscribe((s) => {
      this.rows.set(s);
      this.loaded.set(true);
    });
  }

  acceptRate(r: SessionSummary): number {
    const denom = r.accepts + r.rejects;
    return denom === 0 ? 0 : r.accepts / denom;
  }
}
