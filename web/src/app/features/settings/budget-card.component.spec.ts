// BudgetCardComponent tests using bare TestBed with a stubbed ApiService.
import { importProvidersFrom } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import { Gauge, LucideAngularModule } from 'lucide-angular';
import { BudgetCardComponent } from './budget-card.component';
import { ApiService } from '../../core/api.service';

function setup(saveSpy: (b: { monthly_usd: number }) => unknown = (b) => of(b)) {
  const fakeApi = {
    getSettings: () =>
      of({
        version: 1,
        forwarder: { enabled: false, endpoint: '', timeout_ms: 2000, headers: {} },
        budget: { monthly_usd: 120 },
      }),
    saveBudget: saveSpy,
  };
  TestBed.configureTestingModule({
    imports: [BudgetCardComponent],
    providers: [
      { provide: ApiService, useValue: fakeApi },
      importProvidersFrom(LucideAngularModule.pick({ Gauge })),
    ],
  });
  const fixture = TestBed.createComponent(BudgetCardComponent);
  fixture.detectChanges();
  return { fixture };
}

describe('BudgetCardComponent', () => {
  it('loads the existing budget into the input', () => {
    const { fixture } = setup();
    const input: HTMLInputElement =
      fixture.nativeElement.querySelector('input[type="number"]');
    expect(input.value).toBe('120');
  });

  it('save() sends the entered budget to the API', () => {
    const sent: number[] = [];
    const { fixture } = setup((b) => {
      sent.push(b.monthly_usd);
      return of(b);
    });
    const cmp = fixture.componentInstance;
    cmp.monthly.set(250);
    cmp.dirty.set(true);
    cmp.save();
    expect(sent).toEqual([250]);
  });

  it('typing in the input updates the signal and marks the card dirty', () => {
    const { fixture } = setup();
    const cmp = fixture.componentInstance;
    const input: HTMLInputElement =
      fixture.nativeElement.querySelector('input[type="number"]');
    input.value = '200';
    input.dispatchEvent(new Event('input'));
    expect(cmp.monthly()).toBe(200);
    expect(cmp.dirty()).toBe(true);
  });
});
