import { Component, input } from '@angular/core';

@Component({
  selector: 'app-panel',
  standalone: true,
  template: `
    <section class="bg-panel border border-border rounded-lg p-4 flex flex-col gap-2">
      @if (title()) {
        <h2 class="text-xs uppercase tracking-wider text-muted">{{ title() }}</h2>
      }
      <ng-content />
    </section>
  `,
})
export class PanelComponent {
  title = input<string>('');
}
