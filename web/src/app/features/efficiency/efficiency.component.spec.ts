// EfficiencyComponent tests using bare TestBed with a stubbed ApiService.
import { importProvidersFrom } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import { Calendar, Gauge, Layers, RefreshCw, X, LucideAngularModule } from 'lucide-angular';
import { EfficiencyComponent } from './efficiency.component';
import { ApiService } from '../../core/api.service';

const CACHE = {
  hit_ratio: 0.68,
  hit_ratio_prev: 0.61,
  tokens: { input: 1000, output: 500, cache_read: 3400, cache_create: 700 },
  savings: { net: 42.18, gross: 58.9, creation_overhead: 16.72 },
  net_prev: 35.0,
  unpriced_cache_tokens: 0,
};
const MODELS = [
  {
    family: 'opus',
    role: 'main',
    sessions: 38,
    total_cost_usd: 69.92,
    cost_per_session: 1.84,
    output_tokens: 98480,
    cost_per_1k_output: 0.71,
  },
  {
    family: 'haiku',
    role: 'subagent',
    sessions: 12,
    total_cost_usd: 4.32,
    cost_per_session: 0.36,
    output_tokens: 21000,
    cost_per_1k_output: 0.21,
  },
];

function setup(cache: unknown = CACHE, models: unknown = MODELS) {
  const fakeApi = {
    cacheEfficiency: () => of(cache),
    modelEfficiency: () => of(models),
  };
  TestBed.configureTestingModule({
    imports: [EfficiencyComponent],
    providers: [
      { provide: ApiService, useValue: fakeApi },
      importProvidersFrom(LucideAngularModule.pick({ Gauge, Calendar, Layers, RefreshCw, X })),
    ],
  });
  const fixture = TestBed.createComponent(EfficiencyComponent);
  fixture.detectChanges();
  // Second pass: the data effect runs during the first detectChanges and sets
  // the signals synchronously (of()); this flushes the resulting re-render.
  fixture.detectChanges();
  return { fixture };
}

describe('EfficiencyComponent', () => {
  it('renders the cache hit ratio', () => {
    const { fixture } = setup();
    expect(fixture.nativeElement.textContent).toContain('68%');
  });

  it('renders a model-efficiency row', () => {
    const { fixture } = setup();
    const text = fixture.nativeElement.textContent;
    expect(text).toContain('opus');
    expect(text).toContain('69.92');
  });

  it('shows the empty state when there are no model rows', () => {
    const { fixture } = setup(CACHE, []);
    expect(fixture.nativeElement.textContent).toContain('No data');
  });

  it('shows the unpriced footnote when unpriced_cache_tokens is non-zero', () => {
    const { fixture } = setup({ ...CACHE, unpriced_cache_tokens: 5000 }, MODELS);
    expect(fixture.nativeElement.textContent).toContain('5.0k tokens on un-priced models');
  });

  it('renders a subagent role badge for subagent rows', () => {
    const { fixture } = setup();
    const text = fixture.nativeElement.textContent;
    expect(text).toContain('subagent');
    expect(text).toContain('haiku');
  });
});
