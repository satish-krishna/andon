import { Component, inject } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { FilterService, RangePreset } from '../core/filter.service';

@Component({
  selector: 'app-filter-bar',
  imports: [LucideAngularModule],
  template: `
    <div class="sticky top-0 z-10 bg-panel/90 backdrop-blur-sm border-b border-border">
      <div class="px-6 py-2.5 flex items-center gap-3">
        <span class="filter-label flex items-center gap-1.5">
          <lucide-icon name="calendar" class="w-3 h-3"></lucide-icon>Range
        </span>
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
        <span class="filter-label flex items-center gap-1.5">
          <lucide-icon name="layers" class="w-3 h-3"></lucide-icon>Model
        </span>
        <div class="flex items-center gap-1.5">
          @for (m of filter.allModels(); track m) {
            <button class="filter-chip"
                    [attr.data-active]="filter.models().has(m) ? 'true' : null"
                    (click)="filter.toggleModel(m)">
              {{ m }}
            </button>
          }
        </div>
        @if (filter.hasActiveFilters()) {
          <button class="ml-auto text-muted hover:text-text font-mono text-[11px] flex items-center gap-1"
                  (click)="filter.clearFilters()">
            <lucide-icon name="x" class="w-3 h-3"></lucide-icon>Clear
          </button>
        }
      </div>
    </div>
  `,
})
export class FilterBarComponent {
  filter = inject(FilterService);
  ranges: { id: RangePreset; label: string }[] = [
    { id: 'today', label: 'Today' },
    { id: 'week', label: 'This week' },
    { id: 'month', label: 'This month' },
    { id: '30d', label: 'Last 30d' },
    { id: 'custom', label: 'Custom…' },
  ];
}
