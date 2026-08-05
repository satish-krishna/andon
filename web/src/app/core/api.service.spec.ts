import { TestBed } from '@angular/core/testing';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideHttpClient } from '@angular/common/http';
import { ApiService } from './api.service';

describe('ApiService.saveSweep', () => {
  let api: ApiService;
  let http: HttpTestingController;
  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [ApiService, provideHttpClient(), provideHttpClientTesting()],
    });
    api = TestBed.inject(ApiService);
    http = TestBed.inject(HttpTestingController);
  });
  afterEach(() => http.verify());

  it('PUTs sweep settings to /api/settings/sweep', () => {
    const body = { interval_minutes: 10, enabled: false };
    api.saveSweep(body).subscribe((r) => expect(r.interval_minutes).toBe(10));
    const req = http.expectOne('http://127.0.0.1:8765/api/settings/sweep');
    expect(req.request.method).toBe('PUT');
    expect(req.request.body).toEqual(body);
    req.flush(body);
  });
});
