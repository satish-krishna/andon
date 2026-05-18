// Smoke test for AppComponent: confirms the shell mounts, the version signal
// fills in from ApiService.health(), and the template renders without error.

import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { of } from 'rxjs';
import { importProvidersFrom } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { AppComponent } from './app.component';
import { ApiService } from './core/api.service';
import { APP_ICONS } from './core/icons';

describe('AppComponent', () => {
  it('mounts and populates version from ApiService.health()', async () => {
    const fakeApi: Pick<ApiService, 'health'> = {
      health: () => of({ status: 'ok', version: '0.0.0-test' }) as any,
    };

    TestBed.configureTestingModule({
      imports: [AppComponent],
      providers: [
        provideRouter([]),
        { provide: ApiService, useValue: fakeApi },
        // App template renders lucide icons. Mirror the production registry
        // from app.config.ts so the smoke test fails when an icon is added
        // to the shell template without being added to APP_ICONS.
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
      ],
    });

    const fixture = TestBed.createComponent(AppComponent);
    fixture.detectChanges();
    await fixture.whenStable();

    expect(fixture.componentInstance.version()).toBe('0.0.0-test');
  });
});
