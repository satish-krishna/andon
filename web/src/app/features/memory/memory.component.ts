import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';
import { ApiService } from '../../core/api.service';
import { MemoryEntry, MemoryProject, MemoryTouch } from '../../core/models';

@Component({
  selector: 'app-memory',
  standalone: true,
  imports: [RouterLink, LucideAngularModule],
  templateUrl: './memory.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MemoryComponent implements OnInit {
  private api = inject(ApiService);

  readonly projects = signal<MemoryProject[]>([]);
  readonly slug = signal<string>('');
  readonly entries = signal<MemoryEntry[]>([]);
  /** Raw MEMORY.md text — the index Claude Code loads into context each session. */
  readonly index = signal<string | null>(null);
  readonly showIndex = signal(false);
  /** Seeded true: the page has not verified "no memories" until the first load resolves. */
  readonly loading = signal(true);
  /** True when the last fetch failed. Must never be confused with a genuinely empty result. */
  readonly loadError = signal(false);
  readonly editing = signal<string | null>(null);
  readonly draft = signal('');
  /** Project slug the current edit was started under. Guards against cross-project saves. */
  readonly editingSlug = signal<string | null>(null);
  /** Set when a save or delete fails. Must never be confused with a silent success. */
  readonly actionError = signal<string | null>(null);
  readonly openHistory = signal<string | null>(null);
  /**
   * `| undefined` is load-bearing: an unfetched file's key is genuinely absent at
   * runtime (web/tsconfig.json does not set noUncheckedIndexedAccess), so the `??`
   * fallback in the template on this lookup is real, not dead code.
   */
  readonly history = signal<Record<string, MemoryTouch[] | undefined>>({});

  ngOnInit(): void {
    this.api.memoryProjects().subscribe({
      next: (ps) => {
        this.loadError.set(false);
        this.projects.set(ps);
        if (ps.length > 0) {
          this.select(ps[0].slug);
        } else {
          this.loading.set(false);
        }
      },
      error: () => {
        this.projects.set([]);
        this.loadError.set(true);
        this.loading.set(false);
      },
    });
  }

  /**
   * Owns per-project invalidation. Edit state is scoped to the project it was
   * started under, so it is only stale — and only cleared — when the project
   * actually changes. Re-selecting the SAME project (e.g. the dropdown firing
   * a redundant change event) is not a switch: nothing invalidated, so an
   * in-progress edit survives it, same as a manual refresh now does.
   */
  select(slug: string): void {
    if (this.slug() !== slug) {
      this.editing.set(null);
      this.editingSlug.set(null);
      this.draft.set('');
      this.actionError.set(null);
      this.history.set({});
      this.openHistory.set(null);
    }
    this.slug.set(slug);
    this.refresh();
  }

  /**
   * Matches the sentinel written for UI edits and deletes. Source of truth is
   * `provenance::ANDON_USER` in `src-tauri/src/memory/provenance.rs` — this string
   * literal is not plumbed through the API, so if that constant ever changes, this
   * must be updated by hand or memories edited in Andon will dead-link to
   * `/sessions/andon-user`.
   */
  isAndonUser(t: MemoryTouch): boolean {
    return t.session_id === 'andon-user';
  }

  toggleHistory(file: string): void {
    if (this.openHistory() === file) {
      this.openHistory.set(null);
      return;
    }
    this.openHistory.set(file);
    const slug = this.slug();
    if (!slug) return;
    const requestSlug = slug;
    this.api.memoryTouches(slug, file).subscribe((ts) => {
      if (this.slug() !== requestSlug) return;
      this.history.update((h) => ({ ...h, [file]: ts }));
    });
  }

  toggleIndex(): void {
    this.showIndex.update((v) => !v);
  }

  onProjectChange(event: Event): void {
    this.select((event.target as HTMLSelectElement).value);
  }

  /**
   * Reads from disk on demand. No watcher: memory changes at most once a session.
   *
   * Deliberately does NOT touch `editing`/`editingSlug`/`draft`: this is called both
   * by `select()` (project switch) and by the always-visible manual Refresh button, and
   * a same-project refresh must not vaporize an in-progress edit. Per-project invalidation
   * lives in `select()`, which is the only place the project actually changes.
   *
   * The request's slug is captured locally and checked in BOTH handlers before touching
   * `entries`/`index`/`loadError`/`loading`, so a response that resolves after the user has
   * already switched projects is dropped rather than clobbering the now-current project's
   * state. A dropped response never touches `loading`: the request that "owns" the current
   * project already set `loading` true and will clear it itself when its own response lands.
   */
  refresh(): void {
    const slug = this.slug();
    if (!slug) return;
    this.loading.set(true);
    this.loadError.set(false);
    this.actionError.set(null);
    const requestSlug = slug;
    this.api.memoryList(requestSlug).subscribe({
      next: (r) => {
        if (this.slug() !== requestSlug) return;
        this.entries.set(r.entries);
        this.index.set(r.index);
        this.loading.set(false);
      },
      error: () => {
        if (this.slug() !== requestSlug) return;
        this.entries.set([]);
        this.index.set(null);
        this.loadError.set(true);
        this.loading.set(false);
      },
    });
  }

  /**
   * Drops `file`'s cached history and closes its disclosure if it is open. Called
   * from the SUCCESS paths of `remove()` and `saveEdit()` — the two places THIS APP
   * mutates the provenance ledger (an `andon-user` `delete`/`edit` row). Without
   * this, a stale `history()[file]` cache entry survives the mutation; if the file
   * later reappears (the model reinstating it), the disclosure — if still open —
   * would render the pre-mutation rows, hiding the exact delete-then-recreate churn
   * this feature exists to surface.
   *
   * Scoped to the single touched file, not the whole cache: `remove()`/`saveEdit()`
   * only ever mutate one file, and (per Task 10) `history` is keyed by filename
   * across the whole session, so clearing every other cached file's history here
   * would be needless churn unrelated to this write.
   *
   * Deliberately does NOT refetch inline. If the disclosure for `file` is open,
   * this closes it rather than re-querying: `toggleHistory()` already owns "fetch on
   * open", and closing here means there is exactly one code path that knows how to
   * fetch history, plus the closed state is itself the honest signal that the data
   * underneath just changed — reopening gets a genuinely fresh read.
   *
   * Must NOT be called from `refresh()`: that would defeat the always-visible manual
   * Refresh button's job of leaving history state alone when nothing in this app
   * changed it (see `refresh()`'s own doc comment).
   */
  private invalidateHistory(file: string): void {
    this.history.update((h) => {
      if (!(file in h)) return h;
      const next = { ...h };
      delete next[file];
      return next;
    });
    if (this.openHistory() === file) {
      this.openHistory.set(null);
    }
  }

  private isCurrentDraftDirty(): boolean {
    const file = this.editing();
    if (!file) return false;
    const current = this.entries().find((en) => en.doc.file === file);
    if (!current) return false;
    return this.draft() !== current.doc.raw;
  }

  startEdit(e: MemoryEntry): void {
    if (this.editing() && this.editing() !== e.doc.file && this.isCurrentDraftDirty()) {
      if (!window.confirm(`Discard unsaved changes to ${this.editing()}?`)) return;
    }
    this.editing.set(e.doc.file);
    this.editingSlug.set(this.slug());
    this.draft.set(e.doc.raw);
    this.actionError.set(null);
  }

  cancelEdit(): void {
    this.editing.set(null);
    this.editingSlug.set(null);
    this.draft.set('');
  }

  onDraftInput(event: Event): void {
    this.draft.set((event.target as HTMLTextAreaElement).value);
  }

  saveEdit(file: string): void {
    const slug = this.slug();
    if (!slug) return;
    // Structural guard: the edit must have started under the project we're about to write
    // to. If a future refactor reintroduces a path where stale edit state survives a
    // project switch, this still refuses to let one project's draft land on another's file.
    if (this.editingSlug() !== slug) {
      this.actionError.set(`Save canceled: the project changed since you started editing ${file}.`);
      this.cancelEdit();
      return;
    }
    this.actionError.set(null);
    this.api.memorySave(slug, file, this.draft()).subscribe({
      next: () => {
        this.invalidateHistory(file);
        this.cancelEdit();
        this.refresh();
      },
      error: () => {
        this.actionError.set(`Couldn't save ${file}. Your change was not persisted.`);
      },
    });
  }

  /** Permanent. No undo and no trash: memories are small and self-regenerating. */
  remove(file: string): void {
    const slug = this.slug();
    if (!slug) return;
    if (!window.confirm(`Delete ${file}? This cannot be undone.`)) return;
    this.actionError.set(null);
    this.api.memoryDelete(slug, file).subscribe({
      next: () => {
        this.invalidateHistory(file);
        this.refresh();
        this.refreshProjects();
      },
      error: () => {
        this.actionError.set(`Couldn't delete ${file}. It has not been removed.`);
      },
    });
  }

  /**
   * Re-reads the project list — and therefore each project's memory count — so the
   * switcher reflects a delete without a full page reload. Called only from the
   * delete success path: a delete is the sole in-app action that changes a count
   * (a save does not), and `memory_projects` does a directory listing PER project,
   * so keeping it off `refresh()` (the always-visible manual button) and off save
   * keeps that I/O out of the hot path.
   *
   * Deliberately touches ONLY the global `projects` signal, never
   * `slug`/`entries`/`editing`/`draft`: the list is machine-wide and independent of
   * which project is selected, so applying a late response cannot land one project's
   * state on another. That independence is exactly why this needs none of the
   * request-slug guarding `refresh()` carries.
   *
   * On failure it leaves the existing `projects` list in place — a stale count is a
   * far smaller lie than a blanked switcher — and does NOT raise `loadError`: the
   * delete itself succeeded, so the page is not in an error state.
   */
  private refreshProjects(): void {
    this.api.memoryProjects().subscribe({
      next: (ps) => this.projects.set(ps),
      error: () => {
        /* keep the last-known project list; the delete succeeded regardless */
      },
    });
  }
}
