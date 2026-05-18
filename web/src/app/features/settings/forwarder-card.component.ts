import { CommonModule } from '@angular/common';
import { Component, OnInit, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService, ForwarderSettings } from '../../core/api.service';

interface HeaderRow { key: string; value: string; }

@Component({
  selector: 'app-forwarder-card',
  standalone: true,
  imports: [CommonModule, FormsModule, LucideAngularModule],
  template: `
  <section class="panel" id="forwarder">
    <div class="panel-title">
      <span class="flex items-center gap-1.5">
        <lucide-icon name="share-2" class="w-3.5 h-3.5"></lucide-icon>OTEL forwarder
      </span>
      <label class="inline-flex items-center gap-2 text-[11px] font-mono normal-case tracking-normal">
        <input type="checkbox" [(ngModel)]="enabledModel" (ngModelChange)="onEnabledChange($event)" />
        <span class="text-muted">{{ enabled() ? 'enabled' : 'disabled' }}</span>
      </label>
    </div>
    <div class="panel-body">
      <p class="text-[12px] text-muted mb-3">
        Re-emit every OTLP metric &amp; log Andon receives to a second endpoint (HTTP/protobuf). Default: disabled.
      </p>

      <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
        <label class="text-[11px] font-mono">
          <span class="block text-muted mb-1">endpoint base URL</span>
          <input class="w-full bg-bg border border-border rounded px-2 py-1 text-[12px] font-mono disabled:opacity-50"
                 [(ngModel)]="endpointModel" [disabled]="!enabled()" (ngModelChange)="onEndpointChange($event)"
                 placeholder="https://otel.example.com" />
        </label>
        <label class="text-[11px] font-mono">
          <span class="block text-muted mb-1">timeout (ms)</span>
          <input class="w-full bg-bg border border-border rounded px-2 py-1 text-[12px] font-mono disabled:opacity-50"
                 type="number" min="100" max="30000"
                 [(ngModel)]="timeoutModel" [disabled]="!enabled()" (ngModelChange)="onTimeoutChange($event)" />
        </label>
      </div>

      <div class="mt-4">
        <div class="text-[11px] font-mono text-muted mb-1">headers</div>
        <div class="flex flex-col gap-2">
          @for (row of headers(); track $index) {
            <div class="flex gap-2">
              <input class="flex-1 bg-bg border border-border rounded px-2 py-1 text-[12px] font-mono disabled:opacity-50"
                     placeholder="Header name" [disabled]="!enabled()"
                     [(ngModel)]="row.key" (ngModelChange)="dirty.set(true)" />
              <input class="flex-1 bg-bg border border-border rounded px-2 py-1 text-[12px] font-mono disabled:opacity-50"
                     placeholder="value" [disabled]="!enabled()"
                     [(ngModel)]="row.value" (ngModelChange)="dirty.set(true)" />
              <button class="filter-chip" [disabled]="!enabled()" (click)="removeRow($index)">remove</button>
            </div>
          }
          <button class="filter-chip self-start" [disabled]="!enabled()" (click)="addRow()">+ add header</button>
        </div>
      </div>

      <div class="mt-4 flex items-center gap-2">
        <button class="filter-chip" [disabled]="!dirty()" (click)="save()" [attr.data-active]="dirty() ? 'true' : null">save</button>
        <button class="filter-chip" [disabled]="!enabled()" (click)="test()">test connection</button>
        @if (msg()) {
          <span class="text-[11px] font-mono" [class.text-accent]="ok()" [class.text-err]="err()">{{ msg() }}</span>
        }
      </div>
    </div>
  </section>
  `,
})
export class ForwarderCardComponent implements OnInit {
  private api = inject(ApiService);

  enabled  = signal(false);
  endpoint = signal('');
  timeoutMs = signal(2000);
  headers  = signal<HeaderRow[]>([]);
  dirty    = signal(false);
  msg      = signal('');
  ok       = signal(false);
  err      = signal(false);

  // ngModel proxies
  get enabledModel() { return this.enabled(); }
  set enabledModel(v: boolean) { this.enabled.set(v); }
  get endpointModel() { return this.endpoint(); }
  set endpointModel(v: string) { this.endpoint.set(v); }
  get timeoutModel() { return this.timeoutMs(); }
  set timeoutModel(v: number) { this.timeoutMs.set(v); }

  onEnabledChange(_: boolean) { this.dirty.set(true); }
  onEndpointChange(_: string) { this.dirty.set(true); }
  onTimeoutChange(_: number) { this.dirty.set(true); }

  ngOnInit() {
    this.api.getSettings().subscribe((s) => {
      this.enabled.set(s.forwarder.enabled);
      this.endpoint.set(s.forwarder.endpoint);
      this.timeoutMs.set(s.forwarder.timeout_ms);
      this.headers.set(Object.entries(s.forwarder.headers).map(([key, value]) => ({ key, value })));
      this.dirty.set(false);
    });
  }

  addRow() { this.headers.set([...this.headers(), { key: '', value: '' }]); this.dirty.set(true); }
  removeRow(i: number) {
    const next = [...this.headers()];
    next.splice(i, 1);
    this.headers.set(next);
    this.dirty.set(true);
  }

  private toPayload(): ForwarderSettings {
    const headers: Record<string, string> = {};
    for (const r of this.headers()) if (r.key.trim()) headers[r.key.trim()] = r.value;
    return {
      enabled: this.enabled(),
      endpoint: this.endpoint().trim(),
      timeout_ms: Number(this.timeoutMs()),
      headers,
    };
  }

  save() {
    this.api.saveForwarder(this.toPayload()).subscribe({
      next: () => { this.flash('saved', true); this.dirty.set(false); },
      error: (e) => this.flash(`error: ${e?.error?.error ?? e.message ?? 'failed'}`, false),
    });
  }

  test() {
    this.api.testForwarder(this.toPayload()).subscribe((r) => {
      if (r.ok) this.flash(`ok (HTTP ${r.status})`, true);
      else      this.flash(`failed: ${r.error ?? 'status ' + r.status}`, false);
    });
  }

  private flash(text: string, ok: boolean) {
    this.msg.set(text); this.ok.set(ok); this.err.set(!ok);
    setTimeout(() => { this.msg.set(''); this.ok.set(false); this.err.set(false); }, 4000);
  }
}
