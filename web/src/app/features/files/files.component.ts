import { CommonModule, DatePipe, DecimalPipe } from '@angular/common';
import { Component, computed, effect, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService, V2FileRow, V2FilesResponse } from '../../core/api.service';
import { FilterService } from '../../core/filter.service';
import { FilterBarComponent } from '../../shared/filter-bar.component';

type SortKey = 'edits' | 'accept' | 'recent' | 'churn';

@Component({
  selector: 'app-files',
  standalone: true,
  imports: [CommonModule, DatePipe, DecimalPipe, FormsModule, FilterBarComponent, LucideAngularModule],
  templateUrl: './files.component.html',
})
export class FilesComponent {
  filter = inject(FilterService);
  private api = inject(ApiService);

  data = signal<V2FilesResponse | null>(null);
  sort = signal<SortKey>('edits');
  selectedLangs = signal<Set<string>>(new Set());
  selected = signal<string | null>(null);
  searchInput = '';

  maxEdits = computed(() => {
    const d = this.data();
    if (!d || d.files.length === 0) return 1;
    return Math.max(...d.files.map((f) => f.edits));
  });

  topForHeatmap = computed(() => this.data()?.files.slice(0, 12) ?? []);

  langsSeen = computed(() => {
    const d = this.data();
    if (!d) return [];
    return [...new Set(d.files.map((f) => f.lang))];
  });

  constructor() {
    effect(() => {
      const w = this.filter.window();
      const langs = [...this.selectedLangs()].join(',');
      this.api
        .files({
          fromMs: w.fromMs,
          toMs: w.toMs,
          sort: this.sort(),
          langs: langs || undefined,
          search: this.searchInput || undefined,
        })
        .subscribe((v) => this.data.set(v));
    });
  }

  setSort(k: SortKey) {
    this.sort.set(k);
  }

  toggleLang(l: string) {
    const next = new Set(this.selectedLangs());
    if (next.has(l)) next.delete(l);
    else next.add(l);
    this.selectedLangs.set(next);
  }

  onSearch(v: string) {
    this.searchInput = v;
    const w = this.filter.window();
    const langs = [...this.selectedLangs()].join(',');
    this.api
      .files({
        fromMs: w.fromMs,
        toMs: w.toMs,
        sort: this.sort(),
        langs: langs || undefined,
        search: v || undefined,
      })
      .subscribe((d) => this.data.set(d));
  }

  selectFile(p: string) {
    this.selected.set(this.selected() === p ? null : p);
  }

  acceptColor(rate: number): string {
    if (rate >= 0.85) return '#34d399';
    if (rate >= 0.65) return '#facc15';
    if (rate >= 0.40) return '#f59e0b';
    return '#f87171';
  }

  cellSpan(f: V2FileRow): number {
    const top = this.topForHeatmap();
    const total = top.reduce((a, x) => a + x.edits, 0) || 1;
    return Math.max(1, Math.min(5, Math.round((f.edits / total) * 12)));
  }

  fmtTimeAgo(ts: number): string {
    const diffMin = (Date.now() - ts) / 60000;
    if (diffMin < 60) return `${Math.round(diffMin)}m`;
    if (diffMin < 60 * 24) return `${Math.round(diffMin / 60)}h`;
    return `${Math.round(diffMin / (60 * 24))}d`;
  }

  fileName(p: string): string {
    return p.split(/[/\\]/).pop() || p;
  }
}
