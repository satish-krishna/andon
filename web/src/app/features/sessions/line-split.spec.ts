import { lineSplitSegments } from './line-split';
import { LineSplit } from '../../core/api.service';

function split(
  code: [number, number],
  docs: [number, number],
  other: [number, number],
): LineSplit {
  return {
    code: { added: code[0], removed: code[1] },
    docs: { added: docs[0], removed: docs[1] },
    other: { added: other[0], removed: other[1] },
  };
}

describe('lineSplitSegments', () => {
  it('returns code, docs, other in that order', () => {
    const segs = lineSplitSegments(split([1, 0], [2, 0], [3, 0]));
    expect(segs.map((s) => s.kind)).toEqual(['code', 'docs', 'other']);
  });

  it('computes churn as added + removed', () => {
    const segs = lineSplitSegments(split([10, 4], [0, 0], [0, 0]));
    expect(segs[0].churn).toBe(14);
    expect(segs[0].added).toBe(10);
    expect(segs[0].removed).toBe(4);
  });

  it('computes pct as each bucket share of total churn', () => {
    // total churn = (20+10) + (5+5) + 0 = 40 ; code 75%, docs 25%, other 0%
    const segs = lineSplitSegments(split([20, 10], [5, 5], [0, 0]));
    expect(segs[0].pct).toBeCloseTo(75);
    expect(segs[1].pct).toBeCloseTo(25);
    expect(segs[2].pct).toBe(0);
  });

  it('yields all-zero pct and churn when there is no churn', () => {
    const segs = lineSplitSegments(split([0, 0], [0, 0], [0, 0]));
    expect(segs.every((s) => s.pct === 0)).toBe(true);
    expect(segs.every((s) => s.churn === 0)).toBe(true);
  });
});
