// Reserved for Phase 2/3 specs. Not yet consumed by any spec on this branch —
// see docs/superpowers/plans/2026-05-18-test-harness-phase-2.md for the planned
// callers.
import { Signal } from '@angular/core';

/** Read a signal/computed; convenience to keep test bodies short. */
export function read<T>(s: Signal<T>): T {
  return s();
}
