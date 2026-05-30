// SessionsComponent backfill-flow tests.
// Verifies that clicking "Backfill from file paths" exposes the {scanned, updated}
// result so the missing-repo banner can show the outcome instead of silently
// reloading the list.

import { TestBed } from '@angular/core/testing';
import { ActivatedRoute, provideRouter } from '@angular/router';
import { importProvidersFrom } from '@angular/core';
import { of } from 'rxjs';
import { LucideAngularModule } from 'lucide-angular';

import { SessionsComponent } from './sessions.component';
import { ApiService } from '../../core/api.service';
import { APP_ICONS } from '../../core/icons';

function setup(backfillResponse = { scanned: 5, updated: 3 }) {
  const fakeApi: Partial<ApiService> = {
    sessionsV2: () =>
      of({
        sessions: [],
        coverage: { total: 0, with_repo: 0 },
        totals: {
          cost_usd: 0,
          input_tokens: 0,
          output_tokens: 0,
          accepts: 0,
          rejects: 0,
          aborts: 0,
          lines_added: 0,
          lines_removed: 0,
          lines: [],
        },
      }) as any,
    listRepos: () => of([]) as any,
    backfillRepos: () => of(backfillResponse) as any,
    session: () => of(null) as any,
  };

  TestBed.configureTestingModule({
    imports: [SessionsComponent],
    providers: [
      provideRouter([]),
      { provide: ApiService, useValue: fakeApi },
      { provide: ActivatedRoute, useValue: { snapshot: { queryParamMap: { get: () => null } } } },
      importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
    ],
  });

  const fixture = TestBed.createComponent(SessionsComponent);
  fixture.detectChanges();
  return { fixture, cmp: fixture.componentInstance };
}

describe('SessionsComponent backfill', () => {
  it('exposes the {scanned, updated} result after runBackfill completes', () => {
    const { cmp } = setup({ scanned: 12, updated: 4 });
    cmp.runBackfill();
    expect(cmp.backfillResult()).toEqual({ scanned: 12, updated: 4 });
  });

  it('clears the busy flag after the backfill response arrives', () => {
    const { cmp } = setup();
    cmp.runBackfill();
    expect(cmp.backfillBusy()).toBe(false);
  });
});
