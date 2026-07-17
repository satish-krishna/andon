import { ComponentFixture, TestBed } from '@angular/core/testing';
import { importProvidersFrom } from '@angular/core';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting, HttpTestingController } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';
import { APP_ICONS } from '../../core/icons';
import { SettingsComponent } from './settings.component';

describe('SettingsComponent confirm dialogs', () => {
  let fixture: ComponentFixture<SettingsComponent>;
  let http: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [SettingsComponent],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideRouter([]), importProvidersFrom(LucideAngularModule.pick(APP_ICONS))],
    }).compileComponents();
    fixture = TestBed.createComponent(SettingsComponent);
    http = TestBed.inject(HttpTestingController);
    fixture.detectChanges();
    // Drain the ngOnInit refresh() GETs so expectNone assertions later are clean.
    http.match(() => true).forEach((r) => r.flush(r.request.method === 'GET' ? {} : {}));
  });

  it('opens the dialog for unpatch and posts only on confirm', () => {
    fixture.componentInstance.unpatch();
    expect(fixture.componentInstance.pendingConfirm()).not.toBeNull();
    http.expectNone('http://127.0.0.1:8765/api/integration/unpatch');

    fixture.componentInstance.onConfirm();
    const req = http.expectOne('http://127.0.0.1:8765/api/integration/unpatch');
    req.flush({ ok: true, message: 'unpatched' });
    http.match(() => true).forEach((r) => r.flush({}));
  });

  it('does not post unpatch when the dialog is cancelled', () => {
    fixture.componentInstance.unpatch();
    fixture.componentInstance.onCancel();
    expect(fixture.componentInstance.pendingConfirm()).toBeNull();
    http.expectNone('http://127.0.0.1:8765/api/integration/unpatch');
  });
});
