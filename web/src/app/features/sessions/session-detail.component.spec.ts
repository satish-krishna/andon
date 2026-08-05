// SessionDetailComponent provenance banner test.
// The detail view already shows a small source dot+label next to the Cost panel.
// When a session's cost was reconstructed from local JSONL transcripts (no live
// OTLP telemetry received), we surface an explicit amber banner above the KPI
// grid so the provenance is impossible to miss.

import { TestBed } from '@angular/core/testing';
import { ActivatedRoute, provideRouter } from '@angular/router';
import { of } from 'rxjs';

import { SessionDetailComponent } from './session-detail.component';
import { ApiService } from '../../core/api.service';
import { SessionDetail, SessionSummary } from '../../core/models';

const BANNER_TEXT =
  "This session's data was reconstructed from local transcripts — no live telemetry was received.";

function makeSession(cost_source: SessionSummary['cost_source']): SessionSummary {
  return {
    session_id: 's1',
    started_at: 0,
    ended_at: null,
    cost_usd: 1.23,
    tokens_input: 10,
    tokens_output: 20,
    accepts: 1,
    rejects: 0,
    service_version: null,
    host_arch: null,
    os_type: null,
    cost_source,
  };
}

function makeDetail(cost_source: SessionSummary['cost_source']): SessionDetail {
  return {
    session: makeSession(cost_source),
    cost_by_model: [],
    tokens_by_type: [],
    tool_decisions: [],
    files: [],
    active_time_seconds: 0,
  };
}

function setup(cost_source: SessionSummary['cost_source']) {
  const detail = makeDetail(cost_source);
  const fakeApi: Partial<ApiService> = {
    session: () => of(detail),
    getReport: () => of({ exists: false, path: '', generated_at: null }),
  };

  TestBed.configureTestingModule({
    imports: [SessionDetailComponent],
    providers: [
      provideRouter([]),
      { provide: ApiService, useValue: fakeApi },
      { provide: ActivatedRoute, useValue: { snapshot: { paramMap: { get: () => 's1' } } } },
    ],
  });

  const fixture = TestBed.createComponent(SessionDetailComponent);
  fixture.detectChanges();
  return fixture;
}

describe('SessionDetailComponent provenance banner', () => {
  it('shows the transcript banner when cost_source is jsonl', () => {
    const fixture = setup('jsonl');
    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain(BANNER_TEXT);
  });

  it('hides the transcript banner when cost_source is otlp', () => {
    const fixture = setup('otlp');
    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).not.toContain(BANNER_TEXT);
  });
});
