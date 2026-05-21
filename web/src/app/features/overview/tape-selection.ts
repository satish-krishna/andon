import type { CustomRange, RangePreset } from '../../core/filter.service';

/**
 * The 0-based tape day-bar index that the current filter isolates, or null.
 *
 * Returns an index only when the filter is a single-day `custom` window AND
 * that day falls inside `tapeMonth`. A non-custom range, a multi-day custom
 * range, or a single day outside the displayed month all yield null — no tape
 * day is highlighted.
 *
 * @param range       the filter's current range preset
 * @param filterWindow the filter's resolved window (`filter.window()`)
 * @param tapeMonth   the tape's month as `"YYYY-MM"`, or null before it loads
 */
export function selectedTapeDay(
  range: RangePreset,
  filterWindow: CustomRange,
  tapeMonth: string | null,
): number | null {
  if (range !== 'custom' || tapeMonth === null) return null;
  const from = new Date(filterWindow.fromMs);
  const to = new Date(filterWindow.toMs);
  // A selected day is exactly one calendar day wide.
  if (from.toDateString() !== to.toDateString()) return null;
  const ym = `${from.getFullYear()}-${String(from.getMonth() + 1).padStart(2, '0')}`;
  if (ym !== tapeMonth) return null;
  return from.getDate() - 1;
}

/** The local `Date` for 0-based day `index` of `tapeMonth` (`"YYYY-MM"`). */
export function tapeDayDate(tapeMonth: string, index: number): Date {
  const [year, month] = tapeMonth.split('-').map(Number);
  return new Date(year, month - 1, index + 1);
}
