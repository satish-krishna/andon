import { DecimalPipe } from '@angular/common';
import { Component, effect, inject, input, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { TopRepoEntry } from '../../core/models';

@Component({
  selector: 'app-top-repos-tile',
  standalone: true,
  imports: [DecimalPipe, RouterLink],
  templateUrl: './top-repos-tile.component.html',
})
export class TopReposTileComponent {
  from = input.required<number>();
  to   = input.required<number>();
  private api = inject(ApiService);
  readonly entries = signal<TopRepoEntry[]>([]);

  constructor() {
    effect(() => {
      const from = this.from();
      const to   = this.to();
      this.api.topRepos({ from, to, limit: 5 })
        .subscribe(rows => this.entries.set(rows));
    });
  }

  sparkPoints(spark: number[]): string {
    if (!spark || !spark.length) return '';
    const max = Math.max(...spark, 1);
    const dx = 64 / Math.max(spark.length - 1, 1);
    return spark.map((v, i) => `${i * dx},${24 - (v / max) * 22}`).join(' ');
  }
}
