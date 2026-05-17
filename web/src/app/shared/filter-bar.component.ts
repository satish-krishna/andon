import { Component, inject } from '@angular/core';
import { FilterService, RangePreset } from '../core/filter.service';

@Component({
  selector: 'app-filter-bar',
  template: `
    <div class="sticky top-0 z-10 bg-panel/90 backdrop-blur-sm border-b border-border">
      <div class="px-6 py-2.5 flex items-center gap-3">
        <span class="filter-label">▸ range</span>
        <div class="flex items-center gap-1.5">
          @for (r of ranges; track r.id) {
            <button class="filter-chip"
                    [attr.data-active]="filter.range() === r.id ? 'true' : null"
                    (click)="filter.setRange(r.id)">{{ r.label }}</button>
          }
        </div>
        <span class="ml-auto text-[11px] font-mono text-muted">{{ filter.rangeLabel() }}</span>
      </div>
      <div class="px-6 py-2.5 flex items-center gap-3 border-t border-border/50">
        <span class="filter-label">▸ model</span>
        <div class="flex items-center gap-1.5">
          @for (m of filter.allModels(); track m) {
            <button class="filter-chip"
                    [attr.data-active]="filter.models().has(m) ? 'true' : null"
                    (click)="filter.toggleModel(m)">
              {{ m }}<span class="chip-state">{{ filter.models().has(m) ? 'on' : 'off' }}</span>
            </button>
          }
        </div>
        @if (filter.hasActiveFilters()) {
          <button class="ml-auto text-muted hover:text-text font-mono text-[11px]"
                  (click)="filter.clearFilters()">⊗ clear</button>
        }
      </div>
      <div class="px-6 py-2 text-[11px] uppercase tracking-wider border-t border-border/50 text-muted">
        <span class="inline-block w-14">▸ repo</span>
        <span>— not emitted by claude code · filter by session instead</span>
      </div>
    </div>
  `,
})
export class FilterBarComponent {
  filter = inject(FilterService);
  ranges: { id: RangePreset; label: string }[] = [
    { id: 'today', label: 'today' },
    { id: 'week', label: 'this week' },
    { id: 'month', label: 'this month' },
    { id: '30d', label: 'last 30d' },
    { id: 'custom', label: 'custom…' },
  ];
}
