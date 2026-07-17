import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { MemoryEntry, MemoryProject } from '../../core/models';

@Component({
  selector: 'app-memory',
  standalone: true,
  imports: [RouterLink],
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
  readonly loading = signal(false);

  ngOnInit(): void {
    this.api.memoryProjects().subscribe((ps) => {
      this.projects.set(ps);
      if (ps.length > 0) {
        this.select(ps[0].slug);
      }
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
    this.api.memoryList(slug).subscribe({
      next: (r) => {
        this.entries.set(r.entries);
        this.index.set(r.index);
        this.loading.set(false);
      },
      error: () => {
        this.entries.set([]);
        this.index.set(null);
        this.loading.set(false);
      },
    });
  }
}
