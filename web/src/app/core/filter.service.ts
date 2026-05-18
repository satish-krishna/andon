import { Injectable, computed, signal } from '@angular/core';

export type RangePreset = 'today' | 'week' | 'month' | '30d' | 'custom';

export interface CustomRange {
  fromMs: number;
  toMs: number;
}

// Family tokens; backend matches via substring on the stored full model ID
// (e.g. "claude-opus-4-5-20251001" matches "opus").
const ALL_MODELS = ['opus', 'sonnet', 'haiku'];
const DEFAULT_RANGE: RangePreset = 'month';

@Injectable({ providedIn: 'root' })
export class FilterService {
  readonly range = signal<RangePreset>(DEFAULT_RANGE);
  readonly customRange = signal<CustomRange | null>(null);
  readonly models = signal<Set<string>>(new Set(ALL_MODELS));
  readonly search = signal<string>('');
  readonly repos = signal<string[]>([]);

  readonly window = computed<{ fromMs: number; toMs: number }>(() => {
    const r = this.range();
    if (r === 'custom') {
      const cr = this.customRange();
      if (cr) return cr;
      // Custom selected but no range yet — fall back to current month.
      return monthToToday();
    }
    const now = new Date();
    const todayEnd = endOfDay(now);
    switch (r) {
      case 'today':
        return { fromMs: startOfDay(now).getTime(), toMs: todayEnd.getTime() };
      case 'week': {
        const start = new Date(now);
        const dow = (start.getDay() + 6) % 7; // Monday = 0
        start.setDate(start.getDate() - dow);
        return { fromMs: startOfDay(start).getTime(), toMs: todayEnd.getTime() };
      }
      case 'month':
        return monthToToday();
      case '30d': {
        const start = new Date(now);
        start.setDate(start.getDate() - 29);
        return { fromMs: startOfDay(start).getTime(), toMs: todayEnd.getTime() };
      }
    }
  });

  readonly modelsCsv = computed(() => {
    const s = this.models();
    return s.size === ALL_MODELS.length ? '' : [...s].join(',');
  });

  readonly reposCsv = computed(() => {
    const r = this.repos();
    return r.length ? r.join(',') : '';
  });

  readonly hasActiveFilters = computed(() => {
    return (
      this.range() !== DEFAULT_RANGE ||
      this.models().size !== ALL_MODELS.length ||
      this.search() !== '' ||
      this.repos().length > 0
    );
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

  setRange(r: RangePreset) {
    this.range.set(r);
  }

  enterCustomMode() {
    // Seed customRange from whatever window is currently active so the
    // date inputs are never blank when Custom is first opened.
    const seed = this.window();
    this.customRange.set({ fromMs: seed.fromMs, toMs: seed.toMs });
    this.range.set('custom');
  }

  setCustomFrom(ms: number) {
    const cur = this.customRange() ?? this.window();
    const next = ms > cur.toMs ? { fromMs: cur.toMs, toMs: ms } : { fromMs: ms, toMs: cur.toMs };
    this.customRange.set(next);
  }

  setCustomTo(ms: number) {
    const cur = this.customRange() ?? this.window();
    const next = ms < cur.fromMs ? { fromMs: ms, toMs: cur.fromMs } : { fromMs: cur.fromMs, toMs: ms };
    this.customRange.set(next);
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
    this.range.set(DEFAULT_RANGE);
    this.customRange.set(null);
    this.models.set(new Set(ALL_MODELS));
    this.search.set('');
    this.repos.set([]);
  }

  allModels(): readonly string[] {
    return ALL_MODELS;
  }
}

function monthToToday(): { fromMs: number; toMs: number } {
  const now = new Date();
  const start = new Date(now.getFullYear(), now.getMonth(), 1);
  return { fromMs: start.getTime(), toMs: endOfDay(now).getTime() };
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
