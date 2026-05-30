import { ChangeDetectionStrategy, Component } from '@angular/core';

@Component({
  selector: 'app-andon-mark',
  standalone: true,
  template: `
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" class="block w-full h-full">
      <path d="M 5.03 12.39 A 7 7 0 0 1 7.98 7.27" stroke="#22c55e" stroke-width="2.8" stroke-linecap="round"/>
      <path d="M 9.04 6.66 A 7 7 0 0 1 14.96 6.66" stroke="#f59e0b" stroke-width="2.8" stroke-linecap="round"/>
      <path d="M 16.02 7.27 A 7 7 0 0 1 18.97 12.39" stroke="#ef4444" stroke-width="2.8" stroke-linecap="round"/>
    </svg>
  `,
  styles: `:host { display: inline-block; line-height: 0; }`,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AndonMarkComponent {}
