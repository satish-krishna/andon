// CoachComponent tests using bare TestBed with a stubbed ApiService.
import { importProvidersFrom } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import {
  FlaskConical,
  GraduationCap,
  Layers,
  Lightbulb,
  CircleSlash,
  ChevronDown,
  ChevronRight,
  Calendar,
  RefreshCw,
  X,
  LucideAngularModule,
} from 'lucide-angular';
import { provideRouter } from '@angular/router';
import { CoachComponent } from './coach.component';
import { ApiService } from '../../core/api.service';

const EMPTY_SCORECARD = {
  practices: [],
  window: { from: 0, to: 0 },
  sessions_in_window: 0,
};

const EMPTY_FINDINGS = { items: [], next_cursor: null };

const EMPTY_SKILLS = { lookback: '90d', opportunities: [] };

function setup() {
  const fakeApi = {
    coachScorecard: () => of(EMPTY_SCORECARD),
    coachFindings: () => of(EMPTY_FINDINGS),
    coachSkills: () => of(EMPTY_SKILLS),
  };
  TestBed.configureTestingModule({
    imports: [CoachComponent],
    providers: [
      provideRouter([]),
      { provide: ApiService, useValue: fakeApi },
      importProvidersFrom(
        LucideAngularModule.pick({
          FlaskConical,
          GraduationCap,
          Layers,
          Lightbulb,
          CircleSlash,
          ChevronDown,
          ChevronRight,
          Calendar,
          RefreshCw,
          X,
        }),
      ),
    ],
  });
  const fixture = TestBed.createComponent(CoachComponent);
  fixture.detectChanges();
  fixture.detectChanges();
  return { fixture };
}

describe('CoachComponent', () => {
  it('renders the Coach crumb', () => {
    const { fixture } = setup();
    expect(fixture.nativeElement.textContent).toContain('Coach');
  });

  it('renders the experimental banner', () => {
    const { fixture } = setup();
    expect(fixture.nativeElement.textContent).toContain('Experimental');
  });

  it('renders the Scorecard panel placeholder', () => {
    const { fixture } = setup();
    expect(fixture.nativeElement.textContent).toContain('Scorecard');
  });
});
