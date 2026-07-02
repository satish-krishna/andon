/**
 * True when the tape bar for local calendar date `date` ("YYYY-MM-DD") overlaps
 * the filter window [fromMs, toMs]. Each bar spans its own local day; overlap,
 * not containment, so day-aligned window edges still light the edge bars.
 */
export function dateInWindow(date: string, fromMs: number, toMs: number): boolean {
  const [y, m, d] = date.split('-').map(Number);
  const dayStart = new Date(y, m - 1, d, 0, 0, 0, 0).getTime();
  const dayEnd = new Date(y, m - 1, d, 23, 59, 59, 999).getTime();
  return dayStart <= toMs && dayEnd >= fromMs;
}
