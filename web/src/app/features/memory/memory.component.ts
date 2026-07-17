import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';
import { ApiService } from '../../core/api.service';
import { MemoryEntry, MemoryProject } from '../../core/models';

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
    }
    this.slug.set(slug);
    this.refresh();
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
      next: () => this.refresh(),
      error: () => {
        this.actionError.set(`Couldn't delete ${file}. It has not been removed.`);
      },
    });
  }
}
