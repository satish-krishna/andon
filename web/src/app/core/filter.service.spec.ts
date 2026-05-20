// Bare TestBed pattern — idiomatic for providedIn:'root' services with no
// constructor deps. See filter-bar.component.spec.ts for the component-level
// pattern (also bare TestBed, for the same vitest-angular zone reason).

import { TestBed } from '@angular/core/testing';
import { FilterService } from './filter.service';

function createService(): FilterService {
  TestBed.configureTestingModule({ providers: [FilterService] });
  return TestBed.inject(FilterService);
}

describe('FilterService', () => {
  it('starts on month range with all models selected and no active filters', () => {
    const s = createService();
    expect(s.range()).toBe('month');
    expect(s.models().size).toBe(s.allModels().length);
    expect(s.hasActiveFilters()).toBe(false);
    expect(s.modelsCsv()).toBe(''); // all selected = empty (server convention)
  });

  it('window() for "today" spans start-of-day to end-of-day', () => {
    const s = createService();
    s.setRange('today');
    const w = s.window();
    const from = new Date(w.fromMs);
    const to = new Date(w.toMs);
    expect(from.getHours()).toBe(0);
    expect(to.getHours()).toBe(23);
  });

  it('window() for "30d" spans 30 days inclusive of today', () => {
    const s = createService();
    s.setRange('30d');
    const w = s.window();
    const days = Math.round((w.toMs - w.fromMs) / 86_400_000);
    expect(days).toBeGreaterThanOrEqual(29);
    expect(days).toBeLessThanOrEqual(30);
  });

  it('enterCustomMode seeds the custom range from the prior window', () => {
    const s = createService();
    s.setRange('today');
    const todayWin = s.window();
    s.enterCustomMode();
    expect(s.range()).toBe('custom');
    expect(s.customRange()).toEqual({ fromMs: todayWin.fromMs, toMs: todayWin.toMs });
  });

  it('setCustomFrom clamps so from <= to', () => {
    const s = createService();
    s.enterCustomMode();
    const cur = s.customRange()!;
    s.setCustomFrom(cur.toMs + 1000); // attempt to set "from" past "to"
    const after = s.customRange()!;
    expect(after.fromMs).toBeLessThanOrEqual(after.toMs);
  });

  it('toggleModel refuses to deselect the last active chip', () => {
    const s = createService();
    const all = s.allModels();
    all.slice(1).forEach((m) => s.toggleModel(m)); // remove every chip but the first
    expect(s.models().size).toBe(1);
    s.toggleModel(all[0]); // attempt to remove the last
    expect(s.models().size).toBe(1);
  });

  it('modelsCsv is empty when all selected, csv otherwise', () => {
    const s = createService();
    expect(s.modelsCsv()).toBe('');
    const all = s.allModels();
    s.toggleModel(all[0]); // remove one
    expect(s.modelsCsv().split(',').length).toBe(all.length - 1);
  });

  it('hasActiveFilters reflects range / models / search', () => {
    const s = createService();
    expect(s.hasActiveFilters()).toBe(false);
    s.setRange('today');
    expect(s.hasActiveFilters()).toBe(true);
    s.clearFilters();
    expect(s.hasActiveFilters()).toBe(false);
    s.setSearch('foo');
    expect(s.hasActiveFilters()).toBe(true);
  });

  it('clearFilters resets all state to defaults', () => {
    const s = createService();
    s.setRange('today');
    s.setSearch('foo');
    s.toggleModel(s.allModels()[0]);
    s.clearFilters();
    expect(s.range()).toBe('month');
    expect(s.search()).toBe('');
    expect(s.models().size).toBe(s.allModels().length);
    expect(s.customRange()).toBeNull();
  });

  it('refresh() increments refreshTick and leaves filters untouched', () => {
    const s = createService();
    const before = s.refreshTick();
    s.refresh();
    expect(s.refreshTick()).toBe(before + 1);
    s.refresh();
    expect(s.refreshTick()).toBe(before + 2);
    // refresh() must never behave like clearFilters() — filters stay as they were.
    expect(s.range()).toBe('month');
    expect(s.models().size).toBe(s.allModels().length);
    expect(s.search()).toBe('');
    expect(s.repos()).toEqual([]);
    expect(s.hasActiveFilters()).toBe(false);
  });
});
