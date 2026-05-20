import { describe, it, expect } from 'vitest';

import { sourceLabel, sourceDotClass, sourceTooltip } from './cost-source';

describe('cost-source helpers', () => {
  it('labels each source', () => {
    expect(sourceLabel('otlp')).toBe('OTLP');
    expect(sourceLabel('jsonl')).toBe('JSONL');
    expect(sourceLabel(null)).toBe('—');
  });

  it('maps each source to a dot background class', () => {
    expect(sourceDotClass('otlp')).toBe('bg-ok');
    expect(sourceDotClass('jsonl')).toBe('bg-warn');
    expect(sourceDotClass(null)).toBe('');
  });

  it('describes each source in a tooltip', () => {
    expect(sourceTooltip('otlp')).toBe('Cost from live OpenTelemetry');
    expect(sourceTooltip('jsonl')).toBe('Cost priced retroactively from JSONL transcripts');
    expect(sourceTooltip(null)).toBe('No cost recorded');
  });
});
