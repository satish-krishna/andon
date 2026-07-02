import { dateInWindow } from './tape-window';

// A 1-day window the way FilterService.selectDay builds it.
function dayWindow(year: number, month: number, day: number) {
  return {
    fromMs: new Date(year, month, day, 0, 0, 0, 0).getTime(),
    toMs: new Date(year, month, day, 23, 59, 59, 999).getTime(),
  };
}

describe('dateInWindow', () => {
  it('lights a date inside a multi-day window', () => {
    const w = { fromMs: new Date(2026, 5, 1).getTime(), toMs: new Date(2026, 5, 30, 23, 59, 59, 999).getTime() };
    expect(dateInWindow('2026-06-15', w.fromMs, w.toMs)).toBe(true);
  });

  it('does not light a date before the window', () => {
    const w = dayWindow(2026, 6, 5); // Jul 5 only
    expect(dateInWindow('2026-07-04', w.fromMs, w.toMs)).toBe(false);
  });

  it('does not light a date after the window', () => {
    const w = dayWindow(2026, 6, 5);
    expect(dateInWindow('2026-07-06', w.fromMs, w.toMs)).toBe(false);
  });

  it('lights the single day of a single-day window', () => {
    const w = dayWindow(2026, 6, 5); // month is 0-based => July
    expect(dateInWindow('2026-07-05', w.fromMs, w.toMs)).toBe(true);
  });

  it('lights the last day when the window ends within that day (end-of-day toMs)', () => {
    // window ends at Jul 1 end-of-day (23:59:59.999); the Jul 1 bar must be lit
    const w = { fromMs: new Date(2026, 5, 2).getTime(), toMs: new Date(2026, 6, 1, 23, 59, 59, 999).getTime() };
    expect(dateInWindow('2026-07-01', w.fromMs, w.toMs)).toBe(true);
  });
});
