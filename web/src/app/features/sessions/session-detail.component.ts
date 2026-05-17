import { CommonModule, DatePipe, DecimalPipe } from '@angular/common';
import { Component, OnInit, signal } from '@angular/core';
import { ActivatedRoute, RouterLink } from '@angular/router';

import { ApiService } from '../../core/api.service';
import { SessionDetail } from '../../core/models';
import { PanelComponent } from '../../shared/panel.component';
import { EmptyComponent } from '../../shared/empty.component';

@Component({
  selector: 'app-session-detail',
  standalone: true,
  imports: [CommonModule, DatePipe, DecimalPipe, RouterLink, PanelComponent, EmptyComponent],
  template: `
    <div class="p-6 flex flex-col gap-4">
      <div class="flex items-center gap-3">
        <a routerLink="/sessions" class="text-muted text-xs hover:text-text">← Sessions</a>
        @if (detail(); as d) {
          <h1 class="text-xl font-semibold font-mono">{{ d.session.session_id }}</h1>
        }
      </div>

      @if (detail(); as d) {
        <div class="grid grid-cols-4 gap-4">
          <app-panel title="Started">
            <div class="text-sm">{{ d.session.started_at | date : 'medium' }}</div>
          </app-panel>
          <app-panel title="Cost">
            <div class="text-2xl font-mono">$ {{ d.session.cost_usd | number : '1.4-4' }}</div>
          </app-panel>
          <app-panel title="Tokens">
            <div class="text-sm font-mono">in {{ d.session.tokens_input | number }}</div>
            <div class="text-sm font-mono">out {{ d.session.tokens_output | number }}</div>
          </app-panel>
          <app-panel title="Active time">
            <div class="text-2xl font-mono">{{ fmtDuration(d.active_time_seconds) }}</div>
          </app-panel>
        </div>

        <div class="grid grid-cols-2 gap-4">
          <app-panel title="Cost by model">
            @if (d.cost_by_model.length === 0) {
              <app-empty />
            } @else {
              <ul class="text-sm font-mono space-y-1">
                @for (kv of d.cost_by_model; track kv.key) {
                  <li class="flex justify-between border-b border-border/40 py-1">
                    <span>{{ kv.key }}</span>
                    <span>$ {{ kv.value | number : '1.4-4' }}</span>
                  </li>
                }
              </ul>
            }
          </app-panel>
          <app-panel title="Tokens by type">
            @if (d.tokens_by_type.length === 0) {
              <app-empty />
            } @else {
              <ul class="text-sm font-mono space-y-1">
                @for (kv of d.tokens_by_type; track kv.key) {
                  <li class="flex justify-between border-b border-border/40 py-1">
                    <span>{{ kv.key }}</span>
                    <span>{{ kv.value | number }}</span>
                  </li>
                }
              </ul>
            }
          </app-panel>
        </div>

        <app-panel title="Files touched">
          @if (d.files.length === 0) {
            <app-empty />
          } @else {
            <table class="w-full text-sm font-mono">
              <thead>
                <tr class="text-left text-xs uppercase text-muted">
                  <th class="px-2 py-1">File</th>
                  <th class="px-2 py-1 text-right">+ added</th>
                  <th class="px-2 py-1 text-right">− removed</th>
                </tr>
              </thead>
              <tbody>
                @for (f of d.files; track f.file_path) {
                  <tr class="border-t border-border/40">
                    <td class="px-2 py-1 truncate max-w-md">{{ f.file_path }}</td>
                    <td class="px-2 py-1 text-right text-green-400">+ {{ f.lines_added }}</td>
                    <td class="px-2 py-1 text-right text-red-400">− {{ f.lines_removed }}</td>
                  </tr>
                }
              </tbody>
            </table>
          }
        </app-panel>

        <app-panel title="Tool decisions">
          @if (d.tool_decisions.length === 0) {
            <app-empty />
          } @else {
            <div class="text-xs font-mono max-h-96 overflow-auto">
              @for (t of d.tool_decisions; track $index) {
                <div class="flex gap-3 border-b border-border/40 py-1">
                  <span class="text-muted w-32 shrink-0">{{ t.timestamp | date : 'HH:mm:ss.SSS' }}</span>
                  <span class="w-24 shrink-0">{{ t.tool_name }}</span>
                  <span class="w-16 shrink-0" [class.text-green-400]="t.decision==='accept'"
                                              [class.text-red-400]="t.decision==='reject'"
                                              [class.text-amber-400]="t.decision==='abort'">{{ t.decision }}</span>
                  <span class="text-muted truncate">{{ t.file_path || '' }}</span>
                </div>
              }
            </div>
          }
        </app-panel>
      } @else {
        <app-empty message="Loading…" />
      }
    </div>
  `,
})
export class SessionDetailComponent implements OnInit {
  detail = signal<SessionDetail | null>(null);

  constructor(private route: ActivatedRoute, private api: ApiService) {}

  ngOnInit() {
    const id = this.route.snapshot.paramMap.get('id');
    if (id) {
      this.api.session(id).subscribe((d) => this.detail.set(d));
    }
  }

  fmtDuration(secs: number): string {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
  }
}
