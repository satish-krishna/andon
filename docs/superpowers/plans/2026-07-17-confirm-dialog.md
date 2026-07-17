# Styled Confirm Dialog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace all four native `window.confirm` calls (memory delete + discard-draft, settings unpatch + restore) with one reusable styled `ConfirmDialogComponent`, without reintroducing the cross-project delete race.

**Architecture:** A dumb presentational `ConfirmDialogComponent` (renders a `ConfirmRequest`, emits confirm/cancel) lives in `shared/`. Each host component owns a `pendingConfirm` signal, renders the dialog, and does all logic — including, for the memory delete, capturing `{slug, file}` at open and re-verifying `slug()` at confirm.

**Tech Stack:** Angular 21 (standalone, signals, OnPush), Tailwind 4, Vitest.

## Global Constraints

- Standalone components only. No NgModule. `inject()`, `ChangeDetectionStrategy.OnPush`, `@if`/`@for`.
- Signals for state: `signal()`, `input()`, `output()`, `computed()`. No Subject/Observable in feature code.
- Tailwind utilities mapped to the existing theme tokens (`bg-panel`, `bg-panel-2`, `border-border`, `text-text`, `text-muted`, `text-err`, `bg-err`). No new palette classes, no raw hex.
- No `window.confirm` / `confirm(` may remain in `memory.component.ts` or `settings.component.ts` when done.
- US English. Conventional Commits. TDD (failing test first).
- The memory delete keeps its cross-project guard: `select()` clears `pendingConfirm`; `onConfirm()` re-verifies `slug() === pending.slug` and operates on the captured `pending.slug`, never `this.slug()`. Mirrors the `editingSlug` guard in `saveEdit`.

## File Structure

- Create: `web/src/app/shared/confirm-dialog.component.ts` (exports `ConfirmRequest` and `ConfirmDialogComponent`).
- Create: `web/src/app/shared/confirm-dialog.component.spec.ts`.
- Modify: `web/src/app/features/memory/memory.component.ts`, `memory.component.html`, `memory.component.spec.ts`.
- Modify: `web/src/app/features/settings/settings.component.ts`, `settings.component.html`.
- Create: `web/src/app/features/settings/settings.component.spec.ts`.

---

### Task 1: ConfirmDialogComponent (the primitive)

**Files:**
- Create: `web/src/app/shared/confirm-dialog.component.ts`
- Test: `web/src/app/shared/confirm-dialog.component.spec.ts`

**Interfaces:**
- Produces: `export interface ConfirmRequest { title: string; message: string; confirmLabel: string; danger?: boolean }` and `ConfirmDialogComponent` with `input<ConfirmRequest | null>('request')`, `output<void>('confirm')`, `output<void>('cancel')`. Selector `app-confirm-dialog`.

- [ ] **Step 1: Write the failing spec**

Create `web/src/app/shared/confirm-dialog.component.spec.ts`:
```typescript
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
});
```

- [ ] **Step 2: Run the spec, verify it fails**

Run: `cd web && npx vitest run src/app/shared/confirm-dialog.component.spec.ts`
Expected: FAIL — module `./confirm-dialog.component` does not exist.

- [ ] **Step 3: Implement the component**

Create `web/src/app/shared/confirm-dialog.component.ts`:
```typescript
import { ChangeDetectionStrategy, Component, HostListener, input, output } from '@angular/core';

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
          [attr.aria-label]="r.title"
          (click)="$event.stopPropagation()"
        >
          <div class="px-5 py-4">
            <h2 class="text-sm font-semibold text-text">{{ r.title }}</h2>
            <p class="mt-2 text-sm text-muted">{{ r.message }}</p>
          </div>
          <div class="flex justify-end gap-2 border-t border-border px-5 py-3">
            <button
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

  @HostListener('document:keydown.escape')
  onEscape(): void {
    if (this.request()) this.cancel.emit();
  }
}
```

- [ ] **Step 4: Run the spec, verify it passes**

Run: `cd web && npx vitest run src/app/shared/confirm-dialog.component.spec.ts`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add web/src/app/shared/confirm-dialog.component.ts web/src/app/shared/confirm-dialog.component.spec.ts
git commit -m "feat(ui): add reusable ConfirmDialogComponent"
```

---

### Task 2: Memory delete through the dialog (with the race guard)

**Files:**
- Modify: `web/src/app/features/memory/memory.component.ts`
- Modify: `web/src/app/features/memory/memory.component.html`
- Test: `web/src/app/features/memory/memory.component.spec.ts`

**Interfaces:**
- Consumes: `ConfirmDialogComponent`, `ConfirmRequest` from Task 1.
- Produces: `pendingConfirm` signal, `confirmRequest` computed, `onConfirm()`, `onCancel()`; `remove()` opens the dialog instead of calling `window.confirm`.

- [ ] **Step 1: Write the failing tests**

In `memory.component.spec.ts`, import the dialog and add these tests. (The delete tests that currently mock `window.confirm` are reworked in Step 4; these are the new ones.)
```typescript
it('does not delete against the new project if the user switches projects with the delete dialog open', () => {
  fixture.detectChanges();
  http.expectOne('http://127.0.0.1:8765/api/memory/projects').flush([
    { slug: 'proj-a', label: 'A', count: 1 },
    { slug: 'proj-b', label: 'B', count: 1 },
  ]);
  fixture.detectChanges();
  http.expectOne('http://127.0.0.1:8765/api/memory/proj-a').flush({
    slug: 'proj-a', index: null,
    entries: [{ doc: { file: 'a.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true }, origin: null }],
  });
  fixture.detectChanges();

  // Open the delete dialog for proj-a's file.
  fixture.componentInstance.remove('a.md');
  expect(fixture.componentInstance.pendingConfirm()).not.toBeNull();

  // Switch to proj-b before confirming.
  fixture.componentInstance.select('proj-b');
  fixture.detectChanges();
  http.expectOne('http://127.0.0.1:8765/api/memory/proj-b').flush({ slug: 'proj-b', index: null, entries: [] });

  // The project switch must have closed the dialog...
  expect(fixture.componentInstance.pendingConfirm()).toBeNull();
  // ...and even a stale confirm must not delete anything from proj-b.
  fixture.componentInstance.onConfirm();
  http.expectNone('http://127.0.0.1:8765/api/memory/proj-b/delete');
  http.expectNone('http://127.0.0.1:8765/api/memory/proj-a/delete');
});

it('deletes through the dialog against the captured project on confirm', () => {
  flushProjects();
  http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
    slug: 'D--Repos-andon', index: null,
    entries: [{ doc: { file: 'a.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true }, origin: null }],
  });
  fixture.detectChanges();

  fixture.componentInstance.remove('a.md');
  fixture.componentInstance.onConfirm();

  const req = http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete');
  expect(req.request.body).toEqual({ file: 'a.md' });
  req.flush(null);
  expect(fixture.componentInstance.pendingConfirm()).toBeNull();
  http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({ slug: 'D--Repos-andon', index: null, entries: [] });
  http.expectOne('http://127.0.0.1:8765/api/memory/projects').flush([{ slug: 'D--Repos-andon', label: 'x', count: 0 }]);
});
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts -t "delete dialog open"`
Expected: FAIL — `pendingConfirm` does not exist / `remove` still calls `window.confirm`.

- [ ] **Step 3: Implement in `memory.component.ts`**

Add the import (with the other imports):
```typescript
import { ConfirmDialogComponent, ConfirmRequest } from '../../shared/confirm-dialog.component';
```
Add `ConfirmDialogComponent` to the `@Component({ imports: [...] })` array.

Add a type above the class and the signal/computed inside the class (near the other signals):
```typescript
type PendingConfirm = { kind: 'delete'; slug: string; file: string };
```
```typescript
  readonly pendingConfirm = signal<PendingConfirm | null>(null);
  readonly confirmRequest = computed<ConfirmRequest | null>(() => {
    const p = this.pendingConfirm();
    if (!p) return null;
    // kind === 'delete'
    return {
      title: 'Delete memory?',
      message: `Delete ${p.file}? This is permanent — there is no undo.`,
      confirmLabel: 'Delete',
      danger: true,
    };
  });
```
(Import `computed` from `@angular/core`.)

Replace `remove()` with a version that opens the dialog:
```typescript
  /** Permanent. No undo and no trash: memories are small and self-regenerating. */
  remove(file: string): void {
    const slug = this.slug();
    if (!slug) return;
    this.pendingConfirm.set({ kind: 'delete', slug, file });
  }

  onCancel(): void {
    this.pendingConfirm.set(null);
  }

  onConfirm(): void {
    const p = this.pendingConfirm();
    if (!p) return;
    this.pendingConfirm.set(null);
    // Defense in depth: operate on the captured slug, and refuse if the project
    // changed under the open dialog. Mirrors the editingSlug guard in saveEdit.
    if (this.slug() !== p.slug) return;
    this.actionError.set(null);
    this.api.memoryDelete(p.slug, p.file).subscribe({
      next: () => {
        this.invalidateHistory(p.file);
        this.refresh();
        this.refreshProjects();
      },
      error: () => {
        this.actionError.set(`Couldn't delete ${p.file}. It has not been removed.`);
      },
    });
  }
```

In `select()`, add one line inside the `if (this.slug() !== slug)` block so a project switch closes any open dialog:
```typescript
      this.pendingConfirm.set(null);
```

- [ ] **Step 4: Rework the existing `window.confirm` delete tests**

In `memory.component.spec.ts`, these tests currently drive `window.confirm`. Convert each to the dialog. The mechanical transform: remove the `vi.spyOn(window, 'confirm')...` line; after calling `remove(file)`, either call `fixture.componentInstance.onConfirm()` (to proceed) or `fixture.componentInstance.onCancel()` / assert `pendingConfirm()` (to decline).

- `does not delete when the confirm is declined` → replace with: `fixture.componentInstance.remove('user_role.md'); expect(fixture.componentInstance.pendingConfirm()).not.toBeNull(); fixture.componentInstance.onCancel(); http.expectNone('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete');`
- `posts a delete when the confirm is accepted` → after `remove('user_role.md')`, call `fixture.componentInstance.onConfirm();` then the existing `expectOne(.../delete)` / flush / project re-fetch assertions stay.
- `renders an error when delete fails` → after `remove('user_role.md')`, call `fixture.componentInstance.onConfirm();` then flush the delete with 400 as before.
- `clears the cached history for a file after a successful delete` → after `remove('user_role.md')`, call `fixture.componentInstance.onConfirm();` then the existing delete + projects flushes stay.

Remove any now-unused `vi.spyOn(window, 'confirm')` lines in those four tests.

- [ ] **Step 5: Add the dialog to `memory.component.html`**

At the very end of the template (after the closing `</div>` of the page container), add:
```html
<app-confirm-dialog
  [request]="confirmRequest()"
  (confirm)="onConfirm()"
  (cancel)="onCancel()"
></app-confirm-dialog>
```

- [ ] **Step 6: Run the memory spec, verify it passes**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts`
Expected: all PASS (reworked delete tests + the two new race/happy tests). No `window.confirm` remains in `remove`.

- [ ] **Step 7: Commit**

```bash
git add web/src/app/features/memory/memory.component.ts web/src/app/features/memory/memory.component.html web/src/app/features/memory/memory.component.spec.ts
git commit -m "feat(memory): delete via styled dialog, keeping the cross-project race guard"
```

---

### Task 3: Memory discard-draft through the dialog

**Files:**
- Modify: `web/src/app/features/memory/memory.component.ts`
- Test: `web/src/app/features/memory/memory.component.spec.ts`

**Interfaces:**
- Consumes: `pendingConfirm`, `onConfirm`, `onCancel` from Task 2.
- Produces: `pendingConfirm` widened with a `discard` variant; `startEdit` opens the dialog instead of `window.confirm`; the edit switch moves into `onConfirm`'s discard branch.

- [ ] **Step 1: Write the failing tests**

Rework the two existing discard tests to the dialog and add a discard-confirm test:
```typescript
it('opens the discard dialog when switching edit targets with a dirty draft, and stays on cancel', () => {
  flushProjects();
  http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
    slug: 'D--Repos-andon', index: null,
    entries: [
      { doc: { file: 'a.md', name: null, description: null, kind: null, body: 'a', raw: 'a', parse_ok: true }, origin: null },
      { doc: { file: 'b.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true }, origin: null },
    ],
  });
  fixture.detectChanges();
  const [a, b] = fixture.componentInstance.entries();
  fixture.componentInstance.startEdit(a);
  fixture.componentInstance.draft.set('dirty');

  fixture.componentInstance.startEdit(b);
  // Dialog opened; edit target unchanged until resolved.
  expect(fixture.componentInstance.pendingConfirm()).not.toBeNull();
  expect(fixture.componentInstance.editing()).toBe('a.md');

  fixture.componentInstance.onCancel();
  expect(fixture.componentInstance.editing()).toBe('a.md');
  expect(fixture.componentInstance.draft()).toBe('dirty');
});

it('switches edit target on confirm of the discard dialog', () => {
  flushProjects();
  http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
    slug: 'D--Repos-andon', index: null,
    entries: [
      { doc: { file: 'a.md', name: null, description: null, kind: null, body: 'a', raw: 'a', parse_ok: true }, origin: null },
      { doc: { file: 'b.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true }, origin: null },
    ],
  });
  fixture.detectChanges();
  const [a, b] = fixture.componentInstance.entries();
  fixture.componentInstance.startEdit(a);
  fixture.componentInstance.draft.set('dirty');

  fixture.componentInstance.startEdit(b);
  fixture.componentInstance.onConfirm();
  expect(fixture.componentInstance.editing()).toBe('b.md');
  expect(fixture.componentInstance.draft()).toBe('b');
});
```
Also rework the existing `does not confirm when switching edit targets with an untouched draft` test: it should assert that with a clean draft, `startEdit(b)` switches immediately and `pendingConfirm()` stays null (delete the `vi.spyOn(window,'confirm')` line).

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts -t "discard"`
Expected: FAIL — `startEdit` still calls `window.confirm`; no discard variant.

- [ ] **Step 3: Implement**

Widen the union:
```typescript
type PendingConfirm =
  | { kind: 'delete'; slug: string; file: string }
  | { kind: 'discard'; slug: string; target: MemoryEntry };
```
Extend `confirmRequest`:
```typescript
  readonly confirmRequest = computed<ConfirmRequest | null>(() => {
    const p = this.pendingConfirm();
    if (!p) return null;
    if (p.kind === 'delete') {
      return { title: 'Delete memory?', message: `Delete ${p.file}? This is permanent — there is no undo.`, confirmLabel: 'Delete', danger: true };
    }
    return { title: 'Discard changes?', message: `Discard unsaved changes to ${this.editing()}?`, confirmLabel: 'Discard', danger: true };
  });
```
Replace `startEdit`:
```typescript
  startEdit(e: MemoryEntry): void {
    if (this.editing() && this.editing() !== e.doc.file && this.isCurrentDraftDirty()) {
      this.pendingConfirm.set({ kind: 'discard', slug: this.slug(), target: e });
      return;
    }
    this.applyEdit(e);
  }

  private applyEdit(e: MemoryEntry): void {
    this.editing.set(e.doc.file);
    this.editingSlug.set(this.slug());
    this.draft.set(e.doc.raw);
    this.actionError.set(null);
  }
```
Extend `onConfirm` with the discard branch (before the delete branch's API call):
```typescript
  onConfirm(): void {
    const p = this.pendingConfirm();
    if (!p) return;
    this.pendingConfirm.set(null);
    if (this.slug() !== p.slug) return;
    if (p.kind === 'discard') {
      this.applyEdit(p.target);
      return;
    }
    this.actionError.set(null);
    this.api.memoryDelete(p.slug, p.file).subscribe({
      next: () => { this.invalidateHistory(p.file); this.refresh(); this.refreshProjects(); },
      error: () => { this.actionError.set(`Couldn't delete ${p.file}. It has not been removed.`); },
    });
  }
```

- [ ] **Step 4: Run the memory spec, verify it passes**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts`
Expected: all PASS. Confirm no `window.confirm` remains: `grep -n "window.confirm\|confirm(" web/src/app/features/memory/memory.component.ts || echo clean` → `clean`.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/memory/memory.component.ts web/src/app/features/memory/memory.component.spec.ts
git commit -m "feat(memory): discard-draft via styled dialog"
```

---

### Task 4: Settings unpatch / restore through the dialog

**Files:**
- Modify: `web/src/app/features/settings/settings.component.ts`
- Modify: `web/src/app/features/settings/settings.component.html`
- Test: `web/src/app/features/settings/settings.component.spec.ts` (new)

**Interfaces:**
- Consumes: `ConfirmDialogComponent`, `ConfirmRequest` from Task 1.
- Produces: `pendingConfirm` signal, `confirmRequest` computed, `onConfirm`/`onCancel`; `unpatch()`/`restoreBackup()` open the dialog.

- [ ] **Step 1: Write the failing spec**

Create `web/src/app/features/settings/settings.component.spec.ts`:
```typescript
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { importProvidersFrom } from '@angular/core';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting, HttpTestingController } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';
import { APP_ICONS } from '../../core/icons';
import { SettingsComponent } from './settings.component';

describe('SettingsComponent confirm dialogs', () => {
  let fixture: ComponentFixture<SettingsComponent>;
  let http: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [SettingsComponent],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideRouter([]), importProvidersFrom(LucideAngularModule.pick(APP_ICONS))],
    }).compileComponents();
    fixture = TestBed.createComponent(SettingsComponent);
    http = TestBed.inject(HttpTestingController);
    fixture.detectChanges();
    // Drain the ngOnInit refresh() GETs so expectNone assertions later are clean.
    http.match(() => true).forEach((r) => r.flush(r.request.method === 'GET' ? {} : {}));
  });

  it('opens the dialog for unpatch and posts only on confirm', () => {
    fixture.componentInstance.unpatch();
    expect(fixture.componentInstance.pendingConfirm()).not.toBeNull();
    http.expectNone('http://127.0.0.1:8765/api/integration/unpatch');

    fixture.componentInstance.onConfirm();
    const req = http.expectOne('http://127.0.0.1:8765/api/integration/unpatch');
    req.flush({ ok: true, message: 'unpatched' });
    http.match(() => true).forEach((r) => r.flush({}));
  });

  it('does not post unpatch when the dialog is cancelled', () => {
    fixture.componentInstance.unpatch();
    fixture.componentInstance.onCancel();
    expect(fixture.componentInstance.pendingConfirm()).toBeNull();
    http.expectNone('http://127.0.0.1:8765/api/integration/unpatch');
  });
});
```
NOTE: confirm the unpatch endpoint URL against `api.service.ts` (`unpatchIntegration()`); adjust the URL in the test if it differs.

- [ ] **Step 2: Run the spec, verify it fails**

Run: `cd web && npx vitest run src/app/features/settings/settings.component.spec.ts`
Expected: FAIL — `pendingConfirm`/`onConfirm` do not exist; `unpatch()` still calls `window.confirm`.

- [ ] **Step 3: Implement in `settings.component.ts`**

Add imports:
```typescript
import { computed, signal } from '@angular/core'; // signal already imported; add computed
import { ConfirmDialogComponent, ConfirmRequest } from '../../shared/confirm-dialog.component';
```
Add `ConfirmDialogComponent` to the `@Component({ imports: [...] })` array.

Add state + request:
```typescript
  readonly pendingConfirm = signal<{ kind: 'unpatch' | 'restore' } | null>(null);
  readonly confirmRequest = computed<ConfirmRequest | null>(() => {
    const p = this.pendingConfirm();
    if (!p) return null;
    if (p.kind === 'unpatch') {
      return { title: 'Unpatch settings.json?', message: 'Remove andon env vars from settings.json? Claude Code will stop sending telemetry to andon until you re-apply.', confirmLabel: 'Unpatch', danger: true };
    }
    return { title: 'Restore settings.json?', message: 'Restore the original settings.json from the andon-backup file? Current contents will be overwritten.', confirmLabel: 'Restore', danger: true };
  });
```
Replace `unpatch()` and `restoreBackup()` bodies to open the dialog, and add `onConfirm`/`onCancel`:
```typescript
  unpatch() { this.pendingConfirm.set({ kind: 'unpatch' }); }
  restoreBackup() { this.pendingConfirm.set({ kind: 'restore' }); }
  onCancel() { this.pendingConfirm.set(null); }

  onConfirm() {
    const p = this.pendingConfirm();
    if (!p) return;
    this.pendingConfirm.set(null);
    if (p.kind === 'unpatch') {
      this.api.unpatchIntegration().subscribe((r) => {
        this.flash(r.ok ? (r.message || 'unpatched') : `error: ${r.error}`);
        this.refresh();
      });
    } else {
      this.api.restoreIntegrationBackup().subscribe((r) => {
        this.flash(r.ok ? (r.message || 'restored') : `error: ${r.error}`);
        this.refresh();
      });
    }
  }
```

- [ ] **Step 4: Add the dialog to `settings.component.html`**

At the very end of the template (after the outermost closing `</div>`), add:
```html
<app-confirm-dialog
  [request]="confirmRequest()"
  (confirm)="onConfirm()"
  (cancel)="onCancel()"
></app-confirm-dialog>
```

- [ ] **Step 5: Run the spec, verify it passes**

Run: `cd web && npx vitest run src/app/features/settings/settings.component.spec.ts`
Expected: PASS. Confirm no `window.confirm` remains: `grep -n "window.confirm\|confirm(" web/src/app/features/settings/settings.component.ts || echo clean` → `clean`.

- [ ] **Step 6: Commit**

```bash
git add web/src/app/features/settings/settings.component.ts web/src/app/features/settings/settings.component.html web/src/app/features/settings/settings.component.spec.ts
git commit -m "feat(settings): unpatch/restore via styled dialog"
```

---

### Task 5: Full-suite green and live verification

**Files:** none (verification only).

- [ ] **Step 1: Run the full web suite**

Run: `cd web && npm test`
Expected: all files pass, output pristine (the pre-existing `NG8113 DatePipe` warning is unrelated).

- [ ] **Step 2: Confirm no native confirms remain anywhere**

Run: `grep -rn "window.confirm\|[^.]confirm(" web/src/app/features/memory web/src/app/features/settings || echo clean`
Expected: `clean` (or only matches inside test files / comments — verify none are production `confirm(` calls).

- [ ] **Step 3: Rebuild the SPA and launch**

The dev app serves the prebuilt bundle (see `docs/building.md`) — rebuild + restart to see the change.
```bash
cd web && npm run build
```
Then from the repo root with any running Andon closed: `cargo tauri dev`.

- [ ] **Step 4: Eyeball all four dialogs**

Memory page: Delete a memory → styled dialog, Cancel and Delete both work, Esc/backdrop cancel. Edit a memory, change text, click Edit on another → discard dialog. Settings → Danger zone → Unpatch and Restore → styled dialog. Confirm the dialog matches the dark theme and no native browser confirm appears. Check devtools console is clean.

- [ ] **Step 5: Final commit if any verification fixups were needed**

Only if Step 4 surfaced a fix.

---

## Self-Review

**Spec coverage:**
- Reusable ConfirmDialogComponent → Task 1. ✓
- Memory delete via dialog + double race guard (select clears, onConfirm re-verifies captured slug) → Task 2 Steps 3. ✓
- Memory discard-draft restructure → Task 3. ✓
- Settings unpatch/restore + new spec → Task 4. ✓
- No window.confirm remains → Task 2 Step 6, Task 3 Step 4, Task 4 Step 5, Task 5 Step 2 grep guards. ✓
- Default focus on Cancel → NOTE: the component does not implement programmatic focus-on-open; Esc/backdrop/buttons are covered and tested. If focus-on-open is required, add it in Task 1 via `afterNextRender` + a `viewChild` on the cancel button and verify in the live step (jsdom focus timing is unreliable to unit-test). Flagged for the reviewer to decide whether to require it.
- Theme tokens only → Tailwind classes map to `--color-*`; no raw hex in the component. ✓
- Existing test rework → Task 2 Step 4, Task 3 Step 1. ✓

**Placeholder scan:** No TBD/TODO. Two items require an on-the-spot lookup, both flagged inline: the settings API endpoint URL in Task 4 Step 1 (confirm against `api.service.ts`), and the focus-on-open decision above.

**Type consistency:** `ConfirmRequest` and `ConfirmDialogComponent` from `shared/confirm-dialog.component`; `pendingConfirm`/`confirmRequest`/`onConfirm`/`onCancel` names consistent across Tasks 2–4; `PendingConfirm` union widened in Task 3 matches the `confirmRequest` branches.
