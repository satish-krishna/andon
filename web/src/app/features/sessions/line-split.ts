import { LineSplit } from '../../core/api.service';

export type ChangeKind = 'code' | 'docs' | 'other';

export interface LineSegment {
  kind: ChangeKind;
  added: number;
  removed: number;
  /** added + removed */
  churn: number;
  /** 0-100, this bucket's share of total churn; 0 when there is no churn */
  pct: number;
}

/**
 * Break a LineSplit into three segments (code, docs, other) for the
 * totals-row bar. `pct` is each bucket's share of total churn (added +
 * removed), so the segments tile a 100%-wide bar. When there is no churn at
 * all, every `pct` is 0.
 */
export function lineSplitSegments(split: LineSplit): LineSegment[] {
  const order: ChangeKind[] = ['code', 'docs', 'other'];
  const churnOf = (p: { added: number; removed: number }) => p.added + p.removed;
  const total = churnOf(split.code) + churnOf(split.docs) + churnOf(split.other);
  return order.map((kind) => {
    const pair = split[kind];
    const churn = churnOf(pair);
    return {
      kind,
      added: pair.added,
      removed: pair.removed,
      churn,
      pct: total === 0 ? 0 : (churn / total) * 100,
    };
  });
}
