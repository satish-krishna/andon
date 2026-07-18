import { ChangeDetectionStrategy, Component, ElementRef, HostListener, effect, input, output, viewChild } from '@angular/core';

export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
}

@Component({
  selector: 'app-confirm-dialog',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (request(); as r) {
      <div
        data-role="backdrop"
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
        (click)="cancel.emit()"
      >
        <div
          class="w-full max-w-sm rounded-lg border border-border bg-panel shadow-xl"
          role="dialog"
          aria-modal="true"
          aria-labelledby="confirm-dialog-title"
          aria-describedby="confirm-dialog-message"
          (click)="$event.stopPropagation()"
        >
          <div class="px-5 py-4">
            <h2 id="confirm-dialog-title" class="text-sm font-semibold text-text">{{ r.title }}</h2>
            <p id="confirm-dialog-message" class="mt-2 text-sm text-muted">{{ r.message }}</p>
          </div>
          <div class="flex justify-end gap-2 border-t border-border px-5 py-3">
            <button
              #cancelBtn
              data-role="cancel"
              type="button"
              class="rounded border border-border px-3 py-1.5 text-sm text-text hover:bg-panel-2"
              (click)="cancel.emit()"
            >
              Cancel
            </button>
            <button
              data-role="confirm"
              type="button"
              [class]="
                r.danger
                  ? 'rounded border border-err/40 bg-err/15 px-3 py-1.5 text-sm text-err hover:bg-err/25'
                  : 'rounded bg-text px-3 py-1.5 text-sm text-panel hover:bg-text/80'
              "
              (click)="confirm.emit()"
            >
              {{ r.confirmLabel }}
            </button>
          </div>
        </div>
      </div>
    }
  `,
})
export class ConfirmDialogComponent {
  readonly request = input<ConfirmRequest | null>(null);
  readonly confirm = output<void>();
  readonly cancel = output<void>();

  private readonly cancelBtn = viewChild<ElementRef<HTMLButtonElement>>('cancelBtn');

  constructor() {
    // Move focus to Cancel when the dialog opens: the safe default for a
    // destructive action, so Enter does not fire the confirm. The viewChild
    // signal updates when the @if renders the button, so this effect runs
    // once the element exists.
    effect(() => {
      const btn = this.cancelBtn();
      if (this.request() && btn) btn.nativeElement.focus();
    });
  }

  @HostListener('document:keydown.escape')
  onEscape(): void {
    if (this.request()) this.cancel.emit();
  }
}
