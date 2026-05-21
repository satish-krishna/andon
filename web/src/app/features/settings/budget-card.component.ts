import { CommonModule } from '@angular/common';
import { Component, OnInit, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService } from '../../core/api.service';

@Component({
  selector: 'app-budget-card',
  standalone: true,
  imports: [CommonModule, FormsModule, LucideAngularModule],
  template: `
  <section class="panel" id="budget">
    <div class="panel-title">
      <span class="flex items-center gap-1.5">
        <lucide-icon name="gauge" class="w-3.5 h-3.5"></lucide-icon>Monthly budget
      </span>
    </div>
    <div class="panel-body">
      <p class="text-[12px] text-muted mb-3">
        Set a monthly cost budget. Andon shifts the tray icon to amber at 80% and
        red at 100% of the projected end-of-month spend, and sends one desktop
        notification per threshold. Set to 0 to disable.
      </p>
      <div class="flex items-end gap-3">
        <label class="text-[11px] font-mono">
          <span class="block text-muted mb-1">monthly budget (USD)</span>
          <input class="w-40 bg-bg border border-border rounded px-2 py-1 text-[12px] font-mono"
                 type="number" min="0" max="1000000" step="1"
                 [value]="monthly()"
                 (change)="onInputChange($event)" />
        </label>
        <button class="filter-chip" [disabled]="!dirty()" (click)="save()"
                [attr.data-active]="dirty() ? 'true' : null">save</button>
        @if (msg()) {
          <span class="text-[11px] font-mono pb-1"
                [class.text-accent]="ok()" [class.text-err]="!ok()">{{ msg() }}</span>
        }
      </div>
    </div>
  </section>
  `,
})
export class BudgetCardComponent implements OnInit {
  private api = inject(ApiService);

  monthly = signal(0);
  dirty = signal(false);
  msg = signal('');
  ok = signal(false);

  onInputChange(event: Event) {
    const v = Number((event.target as HTMLInputElement).value);
    this.monthly.set(v);
    this.dirty.set(true);
  }

  ngOnInit() {
    this.api.getSettings().subscribe((s) => {
      this.monthly.set(s.budget.monthly_usd);
      this.dirty.set(false);
    });
  }

  save() {
    this.api.saveBudget({ monthly_usd: Number(this.monthly()) }).subscribe({
      next: () => {
        this.flash('saved', true);
        this.dirty.set(false);
      },
      error: (e) => this.flash(`error: ${e?.error?.error ?? e.message ?? 'failed'}`, false),
    });
  }

  private flash(text: string, ok: boolean) {
    this.msg.set(text);
    this.ok.set(ok);
    setTimeout(() => {
      this.msg.set('');
      this.ok.set(false);
    }, 4000);
  }
}
