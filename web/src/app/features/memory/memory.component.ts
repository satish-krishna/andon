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

  select(slug: string): void {
    this.slug.set(slug);
    this.refresh();
  }

  toggleIndex(): void {
    this.showIndex.update((v) => !v);
  }

  onProjectChange(event: Event): void {
    this.select((event.target as HTMLSelectElement).value);
  }

  /** Reads from disk on demand. No watcher: memory changes at most once a session. */
  refresh(): void {
    const slug = this.slug();
    if (!slug) return;
    this.loading.set(true);
    this.loadError.set(false);
    this.api.memoryList(slug).subscribe({
      next: (r) => {
        this.entries.set(r.entries);
        this.index.set(r.index);
        this.loading.set(false);
      },
      error: () => {
        this.entries.set([]);
        this.index.set(null);
        this.loadError.set(true);
        this.loading.set(false);
      },
    });
  }

  startEdit(e: MemoryEntry): void {
    this.editing.set(e.doc.file);
    this.draft.set(e.doc.raw);
  }

  cancelEdit(): void {
    this.editing.set(null);
    this.draft.set('');
  }

  onDraftInput(event: Event): void {
    this.draft.set((event.target as HTMLTextAreaElement).value);
  }

  saveEdit(file: string): void {
    const slug = this.slug();
    if (!slug) return;
    this.api.memorySave(slug, file, this.draft()).subscribe({
      next: () => {
        this.cancelEdit();
        this.refresh();
      },
    });
  }

  /** Permanent. No undo and no trash: memories are small and self-regenerating. */
  remove(file: string): void {
    const slug = this.slug();
    if (!slug) return;
    if (!window.confirm(`Delete ${file}? This cannot be undone.`)) return;
    this.api.memoryDelete(slug, file).subscribe({ next: () => this.refresh() });
  }
}
