import { Component, inject } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { FilterService, RangePreset } from '../core/filter.service';

@Component({
  selector: 'app-filter-bar',
  imports: [LucideAngularModule],
  template: `
    <div class="sticky top-0 z-10 bg-panel/90 backdrop-blur-sm border-b border-border">
      <div class="px-6 py-2.5 flex items-center gap-3 flex-wrap">
        <span class="filter-label flex items-center gap-1.5">
          <lucide-icon name="calendar" class="w-3 h-3"></lucide-icon>Range
        </span>
        <div class="flex items-center gap-1.5">
          @for (r of ranges; track r.id) {
            <button class="filter-chip"
                    [attr.data-active]="filter.range() === r.id ? 'true' : null"
                    (click)="onRangeClick(r.id)">{{ r.label }}</button>
          }
        </div>
        @if (filter.range() === 'custom') {
          <div class="flex items-center gap-2 ml-1">
            <input type="date"
                   class="filter-date-input"
                   [value]="customFromIso()"
                   (change)="onFromChange($any($event.target).value)" />
            <span class="text-muted text-[11px] font-mono">–</span>
            <input type="date"
                   class="filter-date-input"
                   [value]="customToIso()"
                   (change)="onToChange($any($event.target).value)" />
          </div>
        }
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

  onRangeClick(id: RangePreset) {
    if (id === 'custom') this.filter.enterCustomMode();
    else this.filter.setRange(id);
  }

  onFromChange(iso: string) {
    const ms = parseDateInput(iso, false);
    if (ms !== null) this.filter.setCustomFrom(ms);
  }

  onToChange(iso: string) {
    const ms = parseDateInput(iso, true);
    if (ms !== null) this.filter.setCustomTo(ms);
  }

  customFromIso(): string {
    const cr = this.filter.customRange();
    return cr ? toIsoDate(cr.fromMs) : '';
  }

  customToIso(): string {
    const cr = this.filter.customRange();
    return cr ? toIsoDate(cr.toMs) : '';
  }
}

function parseDateInput(iso: string, endOfDay: boolean): number | null {
  if (!iso) return null;
  const [y, m, d] = iso.split('-').map(Number);
  if (!y || !m || !d) return null;
  const date = new Date(y, m - 1, d);
  if (endOfDay) date.setHours(23, 59, 59, 999);
  else date.setHours(0, 0, 0, 0);
  return date.getTime();
}

function toIsoDate(ms: number): string {
  const d = new Date(ms);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}
