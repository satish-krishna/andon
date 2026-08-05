import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import { SweepCardComponent } from './sweep-card.component';
import { ApiService } from '../../core/api.service';

describe('SweepCardComponent', () => {
  it('saves the current interval and toggle via api.saveSweep', () => {
    const api = {
      getSettings: () => of({ version: 1, forwarder: {}, budget: { monthly_usd: 0 }, sweep: { interval_minutes: 5, enabled: true } }),
      saveSweep: vi.fn(() => of({ interval_minutes: 15, enabled: false })),
    };
    TestBed.configureTestingModule({
      imports: [SweepCardComponent],
      providers: [{ provide: ApiService, useValue: api }],
    });
    const fixture = TestBed.createComponent(SweepCardComponent);
    const cmp = fixture.componentInstance;
    fixture.detectChanges(); // ngOnInit loads settings
    cmp.intervalMinutes.set(15);
    cmp.enabled.set(false);
    cmp.save();
    expect(api.saveSweep).toHaveBeenCalledWith({ interval_minutes: 15, enabled: false });
  });
});
