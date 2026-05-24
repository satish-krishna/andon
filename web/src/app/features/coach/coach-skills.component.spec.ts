import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import { importProvidersFrom } from '@angular/core';
import { LucideAngularModule, GraduationCap, Lightbulb, ChevronRight } from 'lucide-angular';
import { provideRouter } from '@angular/router';
import { CoachSkillsComponent } from './coach-skills.component';
import { ApiService } from '../../core/api.service';

describe('CoachSkillsComponent', () => {
  it('renders the Skill Finder crumb', async () => {
    const fakeApi = {
      coachSkills: () => of({ lookback: '90d', opportunities: [] }),
    };
    await TestBed.configureTestingModule({
      imports: [CoachSkillsComponent],
      providers: [
        importProvidersFrom(LucideAngularModule.pick({ GraduationCap, Lightbulb, ChevronRight })),
        provideRouter([]),
        { provide: ApiService, useValue: fakeApi },
      ],
    }).compileComponents();
    const fix = TestBed.createComponent(CoachSkillsComponent);
    fix.detectChanges();
    expect(fix.nativeElement.textContent).toContain('Skill Finder');
  });
});
