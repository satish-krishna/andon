/** Provenance of a session's cost: live OTLP telemetry, or retroactive JSONL
 *  ingest. `null` when the session has no cost rows at all. */
export type CostSource = 'otlp' | 'jsonl' | null;

/** Short column label for a cost source. */
export function sourceLabel(s: CostSource): string {
  if (s === 'otlp') return 'OTLP';
  if (s === 'jsonl') return 'JSONL';
  return '—';
}

/** Tailwind background class for the source indicator dot. */
export function sourceDotClass(s: CostSource): string {
  if (s === 'otlp') return 'bg-ok';
  if (s === 'jsonl') return 'bg-warn';
  return '';
}

/** Hover text explaining where a session's cost figure came from. */
export function sourceTooltip(s: CostSource): string {
  if (s === 'otlp') return 'Cost from live OpenTelemetry';
  if (s === 'jsonl') return 'Cost priced retroactively from JSONL transcripts';
  return 'No cost recorded';
}
