// FilterBarComponent tests using bare TestBed.
//
// Adaptation from plan: replaced createComponentFactory (Spectator) with
// TestBed.configureTestingModule + TestBed.createComponent because
// @ngneat/spectator's factory helpers are incompatible with @analogjs/vitest-angular's
// zone setup on Angular 21. See spectator-factories.ts for details.
//
// Selector adaptations from plan literals:
//   - Range chip text: "Today" (capital T), not "today" — located via case-insensitive
//     startsWith on trimmed textContent.
//   - "Custom…" chip text contains "Custom" (with ellipsis); located via includes().
//   - Model chips share the same ".filter-chip" class as range chips; distinguished by
//     matching textContent against filter.allModels() values.
//   - Clear button: no data-clear attribute or aria-label. Located via button textContent
//     containing "Clear" inside the component host.
//   - hasActiveFilters() is a computed signal, not a plain boolean method call — invoked
//     as filter.hasActiveFilters() per FilterService public API.

import { importProvidersFrom } from '@angular/core';
import { TestBed, ComponentFixture } from '@angular/core/testing';
import { Calendar, Layers, X, LucideAngularModule } from 'lucide-angular';
import { FilterBarComponent } from './filter-bar.component';
import { FilterService } from '../core/filter.service';

function setup(): { fixture: ComponentFixture<FilterBarComponent>; filter: FilterService } {
  TestBed.configureTestingModule({
    imports: [FilterBarComponent],
    providers: [
      FilterService,
      // Provide only the icons used by FilterBarComponent (calendar, layers, x).
      // Without this, LucideAngularComponent throws on ngOnChanges.
      importProvidersFrom(LucideAngularModule.pick({ Calendar, Layers, X })),
    ],
  });
  const fixture = TestBed.createComponent(FilterBarComponent);
  fixture.detectChanges();
  const filter = TestBed.inject(FilterService);
  return { fixture, filter };
}

describe('FilterBarComponent', () => {
  it('clicking a range chip switches the preset', () => {
    const { fixture, filter } = setup();
    const chips = Array.from(
      fixture.nativeElement.querySelectorAll('.filter-chip')
    ) as HTMLElement[];
    const todayChip = chips.find((b) =>
      b.textContent?.trim().toLowerCase().startsWith('today')
    );
    expect(todayChip).toBeTruthy();
    todayChip!.click();
    fixture.detectChanges();
    expect(filter.range()).toBe('today');
  });

  it('clicking "Custom…" reveals the date inputs', () => {
    const { fixture } = setup();
    // No date inputs before Custom is selected (default range is "month")
    expect(fixture.nativeElement.querySelectorAll('input[type="date"]').length).toBe(0);
    const chips = Array.from(
      fixture.nativeElement.querySelectorAll('.filter-chip')
    ) as HTMLElement[];
    const custom = chips.find((b) =>
      b.textContent?.toLowerCase().includes('custom')
    );
    expect(custom).toBeTruthy();
    custom!.click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelectorAll('input[type="date"]').length).toBe(2);
  });

  it('toggling a model chip updates the service', () => {
    const { fixture, filter } = setup();
    // All three models are selected by default; clicking the first removes it.
    const firstModel = filter.allModels()[0];
    const chips = Array.from(
      fixture.nativeElement.querySelectorAll('.filter-chip')
    ) as HTMLElement[];
    const chip = chips.find((b) => b.textContent?.trim() === firstModel);
    expect(chip).toBeTruthy();
    chip!.click();
    fixture.detectChanges();
    expect(filter.models().has(firstModel)).toBe(false);
  });

  it('Clear button is hidden when no active filters and visible when any', () => {
    const { fixture, filter } = setup();
    // Default state: range = "month", all models selected, no search → no active filters
    const clearBtn = (): HTMLElement | null => {
      const buttons = Array.from(
        fixture.nativeElement.querySelectorAll('button')
      ) as HTMLElement[];
      return buttons.find((b) => b.textContent?.includes('Clear')) ?? null;
    };
    expect(clearBtn()).toBeFalsy();
    // Activating a search filter (does not touch DOM — mutates service signal directly)
    filter.setSearch('foo');
    fixture.detectChanges();
    expect(clearBtn()).toBeTruthy();
  });
});
