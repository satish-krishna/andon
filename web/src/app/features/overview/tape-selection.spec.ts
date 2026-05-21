import { selectedTapeDay, tapeDayDate } from './tape-selection';

// Build a 1-day window the way FilterService.selectDay does, for the given
// local calendar day (month is 0-based, matching the Date constructor).
function dayWindow(year: number, month: number, day: number) {
  return {
    fromMs: new Date(year, month, day, 0, 0, 0, 0).getTime(),
    toMs: new Date(year, month, day, 23, 59, 59, 999).getTime(),
  };
}

describe('selectedTapeDay', () => {
  it('returns the 0-based index for a single-day custom window in the tape month', () => {
    expect(selectedTapeDay('custom', dayWindow(2026, 4, 15), '2026-05')).toBe(14);
  });

  it('returns null when the range is not custom', () => {
    expect(selectedTapeDay('month', dayWindow(2026, 4, 15), '2026-05')).toBeNull();
  });

  it('returns null for a multi-day custom window', () => {
    const w = {
      fromMs: new Date(2026, 4, 1).getTime(),
      toMs: new Date(2026, 4, 20).getTime(),
    };
    expect(selectedTapeDay('custom', w, '2026-05')).toBeNull();
  });

  it('returns null when the selected day is outside the tape month', () => {
    // April 15 selected while the tape shows May
    expect(selectedTapeDay('custom', dayWindow(2026, 3, 15), '2026-05')).toBeNull();
  });

  it('returns null when the tape month has not loaded', () => {
    expect(selectedTapeDay('custom', dayWindow(2026, 4, 15), null)).toBeNull();
  });

  it('treats a point-in-time window (fromMs === toMs) as a single-day selection', () => {
    const ms = new Date(2026, 4, 15, 10, 30, 0, 0).getTime();
    expect(selectedTapeDay('custom', { fromMs: ms, toMs: ms }, '2026-05')).toBe(14);
  });

  it('handles a single-digit month correctly', () => {
    expect(selectedTapeDay('custom', dayWindow(2026, 0, 1), '2026-01')).toBe(0);
  });
});

describe('tapeDayDate', () => {
  it('builds the local date for a 0-based day index of the tape month', () => {
    const d = tapeDayDate('2026-05', 14);
    expect(d.getFullYear()).toBe(2026);
    expect(d.getMonth()).toBe(4); // May, 0-based
    expect(d.getDate()).toBe(15);
  });
});
