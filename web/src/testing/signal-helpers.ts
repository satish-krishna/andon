import { Signal } from '@angular/core';

/** Read a signal/computed; convenience to keep test bodies short. */
export function read<T>(s: Signal<T>): T {
  return s();
}
