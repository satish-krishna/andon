import { Injectable, computed, signal } from '@angular/core';

export type RangePreset = 'today' | 'week' | 'month' | '30d' | 'custom';

export interface CustomRange {
  fromMs: number;
  toMs: number;
}

const ALL_MODELS = ['opus', 'sonnet', 'haiku'];

@Injectable({ providedIn: 'root' })
export class FilterService {
  // ----- state -----
  readonly range = signal<RangePreset>('month');
  readonly customRange = signal<CustomRange | null>(null);
  readonly models = signal<Set<string>>(new Set(ALL_MODELS));
  readonly search = signal<string>('');

  // ----- derived -----
  readonly window = computed<{ fromMs: number; toMs: number }>(() => {
    const r = this.range();
    if (r === 'custom' && this.customRange()) return this.customRange()!;
    const now = new Date();
    const todayEnd = endOfDay(now);
    switch (r) {
      case 'today':
        return { fromMs: startOfDay(now).getTime(), toMs: todayEnd.getTime() };
      case 'week': {
        const start = new Date(now);
        const dow = (start.getDay() + 6) % 7; // monday = 0
        start.setDate(start.getDate() - dow);
        return { fromMs: startOfDay(start).getTime(), toMs: todayEnd.getTime() };
      }
      case 'month': {
        const start = new Date(now.getFullYear(), now.getMonth(), 1);
        return { fromMs: start.getTime(), toMs: todayEnd.getTime() };
      }
      case '30d': {
        const start = new Date(now);
        start.setDate(start.getDate() - 29);
        return { fromMs: startOfDay(start).getTime(), toMs: todayEnd.getTime() };
      }
      default:
        return { fromMs: 0, toMs: todayEnd.getTime() };
    }
  });

  readonly modelsCsv = computed(() => {
    const s = this.models();
    return s.size === ALL_MODELS.length ? '' : [...s].join(',');
  });

  readonly hasActiveFilters = computed(() => {
    return this.models().size !== ALL_MODELS.length || this.search() !== '';
  });

  readonly rangeLabel = computed(() => {
    const r = this.range();
    const w = this.window();
    const from = new Date(w.fromMs);
    const to = new Date(w.toMs);
    const fmt = (d: Date) => d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    switch (r) {
      case 'today':
        return `today · ${fmt(from)}`;
      case 'week':
        return `this week · ${fmt(from)} – today`;
      case 'month': {
        const monthName = from.toLocaleDateString(undefined, { month: 'long' });
        const dayOfMonth = new Date().getDate();
        const daysInMonth = new Date(from.getFullYear(), from.getMonth() + 1, 0).getDate();
        return `${monthName} · day ${dayOfMonth} of ${daysInMonth}`;
      }
      case '30d':
        return `last 30d · ${fmt(from)} – ${fmt(to)}`;
      case 'custom':
        return `custom · ${fmt(from)} – ${fmt(to)}`;
    }
  });

  // ----- actions -----
  setRange(r: RangePreset) {
    this.range.set(r);
  }

  toggleModel(m: string) {
    const next = new Set(this.models());
    if (next.has(m)) next.delete(m);
    else next.add(m);
    this.models.set(next);
  }

  setSearch(s: string) {
    this.search.set(s);
  }

  clearFilters() {
    this.models.set(new Set(ALL_MODELS));
    this.search.set('');
  }

  allModels(): readonly string[] {
    return ALL_MODELS;
  }
}

function startOfDay(d: Date): Date {
  const x = new Date(d);
  x.setHours(0, 0, 0, 0);
  return x;
}
function endOfDay(d: Date): Date {
  const x = new Date(d);
  x.setHours(23, 59, 59, 999);
  return x;
}
