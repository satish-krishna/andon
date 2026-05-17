import { CommonModule, DecimalPipe, PercentPipe } from '@angular/common';
import { Component, OnInit, computed, signal } from '@angular/core';

import { ApiService } from '../../core/api.service';
import { FileHeatmapRow } from '../../core/models';
import { PanelComponent } from '../../shared/panel.component';
import { EmptyComponent } from '../../shared/empty.component';

@Component({
    selector: 'app-files',
    imports: [CommonModule, DecimalPipe, PercentPipe, PanelComponent, EmptyComponent],
    template: `
    <div class="p-6 flex flex-col gap-4">
      <h1 class="text-xl font-semibold">Files</h1>

      <app-panel title="Edit heatmap (last 30 days)">
        @if (rows().length === 0) {
          <app-empty />
        } @else {
          <div class="grid grid-cols-12 gap-1 max-h-[600px] overflow-auto">
            @for (r of rows(); track r.file_path) {
              <div class="col-span-12 grid grid-cols-12 gap-2 items-center text-xs font-mono py-1 border-b border-border/30">
                <div class="col-span-7 truncate" [title]="r.file_path">{{ r.file_path }}</div>
                <div class="col-span-2 text-right">{{ r.edit_count | number }}</div>
                <div class="col-span-3 h-3 rounded-sm"
                     [style.background]="cellColor(r)"></div>
              </div>
            }
          </div>
        }
      </app-panel>
    </div>
  `
})
export class FilesComponent implements OnInit {
  rows = signal<FileHeatmapRow[]>([]);
  maxEdits = computed(() => Math.max(1, ...this.rows().map((r) => r.edit_count)));

  constructor(private api: ApiService) {}

  ngOnInit() {
    this.api.filesHeatmap(30).subscribe((r) => this.rows.set(r));
  }

  cellColor(r: FileHeatmapRow): string {
    const intensity = r.edit_count / this.maxEdits();
    const alpha = 0.15 + intensity * 0.85;
    // hue: red (0) for low accept, green (120) for high
    const hue = Math.round(r.accept_rate * 120);
    return `hsla(${hue}, 80%, 50%, ${alpha})`;
  }
}
