import { ComponentFixture, TestBed } from '@angular/core/testing';
import { ConfirmDialogComponent } from './confirm-dialog.component';

describe('ConfirmDialogComponent', () => {
  let fixture: ComponentFixture<ConfirmDialogComponent>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({ imports: [ConfirmDialogComponent] }).compileComponents();
    fixture = TestBed.createComponent(ConfirmDialogComponent);
  });

  function el(): HTMLElement { return fixture.nativeElement as HTMLElement; }

  it('renders nothing when request is null', () => {
    fixture.componentRef.setInput('request', null);
    fixture.detectChanges();
    expect(el().querySelector('[role="dialog"]')).toBeNull();
  });

  it('renders title, message and confirm label when request is set', () => {
    fixture.componentRef.setInput('request', { title: 'Delete memory?', message: 'Gone for good.', confirmLabel: 'Delete', danger: true });
    fixture.detectChanges();
    const text = el().textContent ?? '';
    expect(el().querySelector('[role="dialog"]')).toBeTruthy();
    expect(text).toContain('Delete memory?');
    expect(text).toContain('Gone for good.');
    expect(text).toContain('Delete');
  });

  it('describes the dialog with the message via aria-describedby', () => {
    fixture.componentRef.setInput('request', { title: 'Delete memory?', message: 'Gone for good.', confirmLabel: 'Delete', danger: true });
    fixture.detectChanges();
    const dialog = el().querySelector('[role="dialog"]')!;
    const describedById = dialog.getAttribute('aria-describedby');
    expect(describedById).toBeTruthy();
    const describedBy = document.getElementById(describedById!);
    expect(describedBy?.textContent).toContain('Gone for good.');
  });

  it('emits confirm when the confirm button is clicked', () => {
    fixture.componentRef.setInput('request', { title: 't', message: 'm', confirmLabel: 'Delete' });
    fixture.detectChanges();
    let confirmed = false;
    fixture.componentInstance.confirm.subscribe(() => (confirmed = true));
    el().querySelector<HTMLButtonElement>('[data-role="confirm"]')!.click();
    expect(confirmed).toBe(true);
  });

  it('emits cancel on the cancel button, on Escape, and on backdrop click', () => {
    fixture.componentRef.setInput('request', { title: 't', message: 'm', confirmLabel: 'Delete' });
    fixture.detectChanges();
    let cancels = 0;
    fixture.componentInstance.cancel.subscribe(() => cancels++);

    el().querySelector<HTMLButtonElement>('[data-role="cancel"]')!.click();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    el().querySelector<HTMLElement>('[data-role="backdrop"]')!.click();

    expect(cancels).toBe(3);
  });

  it('moves focus to the cancel button when the dialog opens', () => {
    fixture.componentRef.setInput('request', { title: 't', message: 'm', confirmLabel: 'Delete', danger: true });
    fixture.detectChanges();
    // The focus effect runs once the @if renders the button; if activeElement is
    // not yet the button, a second detectChanges() flushes the viewChild→effect tick.
    fixture.detectChanges();
    expect(document.activeElement).toBe(el().querySelector('[data-role="cancel"]'));
  });
});
