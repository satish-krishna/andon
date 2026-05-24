import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import { importProvidersFrom } from '@angular/core';
import { LucideAngularModule, GraduationCap, CircleSlash } from 'lucide-angular';
import { CoachCardComponent } from './coach-card.component';
import { ApiService } from '../../core/api.service';

describe('CoachCardComponent', () => {
  async function setup() {
    const fakeApi = {
      coachSettings: () => of({
        skill_min_occurrences: 3, skill_min_sessions: 2,
        planning_commands: ['plan'],
        planning_keywords: ['spec'],
        constraint_keywords: ['must'],
      }),
      coachRules: () => of([]),
      saveCoachSettings: (s: any) => of(s),
      updateCoachRule: (_id: string, _enabled: boolean) => of({ ok: true }),
    };
    await TestBed.configureTestingModule({
      imports: [CoachCardComponent],
      providers: [
        importProvidersFrom(LucideAngularModule.pick({ GraduationCap, CircleSlash })),
        { provide: ApiService, useValue: fakeApi },
      ],
    }).compileComponents();
    return TestBed.createComponent(CoachCardComponent);
  }

  it('renders Coach header', async () => {
    const fix = await setup();
    fix.detectChanges();
    expect(fix.nativeElement.textContent).toContain('Coach');
  });
});
