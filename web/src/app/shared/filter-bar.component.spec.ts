// FilterBarComponent tests using bare TestBed.
//
// Note: @ngneat/spectator's createComponentFactory is incompatible with
// @analogjs/vitest-angular's zone setup on Angular 21 (zone cleanup nulls
// the compiler between configureTestingModule and inject). Use bare TestBed
// + TestBed.createComponent instead.

import { importProvidersFrom } from '@angular/core';
import { TestBed, ComponentFixture } from '@angular/core/testing';
import { Calendar, Layers, RefreshCw, X, LucideAngularModule } from 'lucide-angular';
import { FilterBarComponent } from './filter-bar.component';
import { FilterService } from '../core/filter.service';

function setup(): { fixture: ComponentFixture<FilterBarComponent>; filter: FilterService } {
  TestBed.configureTestingModule({
    imports: [FilterBarComponent],
    providers: [
      FilterService,
      // Provide only the icons used by FilterBarComponent (calendar, layers, x).
      // Without this, LucideAngularComponent throws on ngOnChanges.
      importProvidersFrom(LucideAngularModule.pick({ Calendar, Layers, RefreshCw, X })),
    ],
  });
  const fixture = TestBed.createComponent(FilterBarComponent);
  fixture.detectChanges();
  const filter = TestBed.inject(FilterService);
  return { fixture, filter };
}

function rangeChip(fixture: ComponentFixture<FilterBarComponent>, id: string): HTMLElement | null {
  return fixture.nativeElement.querySelector(`[data-chip="range"][data-chip-id="${id}"]`);
}

function modelChip(fixture: ComponentFixture<FilterBarComponent>, id: string): HTMLElement | null {
  return fixture.nativeElement.querySelector(`[data-chip="model"][data-chip-id="${id}"]`);
}

function clearButton(fixture: ComponentFixture<FilterBarComponent>): HTMLElement | null {
  return fixture.nativeElement.querySelector('[data-testid="clear-filters"]');
}

function refreshButton(fixture: ComponentFixture<FilterBarComponent>): HTMLElement | null {
  return fixture.nativeElement.querySelector('[data-testid="refresh-data"]');
}

describe('FilterBarComponent', () => {
  it('clicking a range chip switches the preset', () => {
    const { fixture, filter } = setup();
    const today = rangeChip(fixture, 'today');
    expect(today).toBeTruthy();
    today!.click();
    fixture.detectChanges();
    expect(filter.range()).toBe('today');
  });

  it('clicking "Custom…" reveals the date inputs', () => {
    const { fixture } = setup();
    // No date inputs before Custom is selected (default range is "month")
    expect(fixture.nativeElement.querySelectorAll('input[type="date"]').length).toBe(0);
    const custom = rangeChip(fixture, 'custom');
    expect(custom).toBeTruthy();
    custom!.click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelectorAll('input[type="date"]').length).toBe(2);
  });

  it('toggling a model chip updates the service', () => {
    const { fixture, filter } = setup();
    // All three models are selected by default; clicking the first removes it.
    const firstModel = filter.allModels()[0];
    const chip = modelChip(fixture, firstModel);
    expect(chip).toBeTruthy();
    chip!.click();
    fixture.detectChanges();
    expect(filter.models().has(firstModel)).toBe(false);
  });

  it('Clear button is hidden when no active filters and visible when any', () => {
    const { fixture, filter } = setup();
    // Default state: range = "month", all models selected, no search → no active filters
    expect(clearButton(fixture)).toBeFalsy();
    // Activating a search filter (does not touch DOM — mutates service signal directly)
    filter.setSearch('foo');
    fixture.detectChanges();
    expect(clearButton(fixture)).toBeTruthy();
  });

  it('Refresh button is always visible, including with no active filters', () => {
    const { fixture } = setup();
    // Default state has no active filters (Clear is hidden) — Refresh still shows.
    expect(clearButton(fixture)).toBeFalsy();
    expect(refreshButton(fixture)).toBeTruthy();
  });

  it('clicking Refresh bumps refreshTick without activating any filter', () => {
    const { fixture, filter } = setup();
    const btn = refreshButton(fixture);
    expect(btn).toBeTruthy();
    const before = filter.refreshTick();
    btn!.click();
    fixture.detectChanges();
    expect(filter.refreshTick()).toBe(before + 1);
    expect(filter.hasActiveFilters()).toBe(false);
  });
});
