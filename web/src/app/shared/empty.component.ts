import { Component, input } from '@angular/core';

@Component({
  selector: 'app-empty',
  standalone: true,
  template: `
    <div class="flex items-center justify-center text-muted text-xs py-10">
      {{ message() }}
    </div>
  `,
})
export class EmptyComponent {
  message = input<string>('No data yet — start using Claude Code.');
}
