import { CommonModule, DecimalPipe, PercentPipe } from '@angular/common';
import { Component, OnInit, signal } from '@angular/core';
import { BaseChartDirective } from 'ng2-charts';
import { ChartConfiguration } from 'chart.js';

import { ApiService } from '../../core/api.service';
import {
  AcceptByLanguage,
  ActiveTimeToday,
  DailySeries,
  OverviewToday,
} from '../../core/models';
import { PanelComponent } from '../../shared/panel.component';
import { EmptyComponent } from '../../shared/empty.component';

const MODEL_COLORS = [
  '#facc15', '#60a5fa', '#a78bfa', '#34d399', '#f472b6', '#fb923c', '#22d3ee',
];

@Component({
  selector: 'app-overview',
  standalone: true,
  imports: [
    CommonModule,
    DecimalPipe,
    PercentPipe,
    BaseChartDirective,
    PanelComponent,
    EmptyComponent,
  ],
  templateUrl: './overview.component.html',
})
export class OverviewComponent implements OnInit {
  today = signal<OverviewToday | null>(null);
  activeTime = signal<ActiveTimeToday | null>(null);
  accept = signal<AcceptByLanguage[]>([]);

  costChart = signal<ChartConfiguration<'bar'> | null>(null);
  tokensChart = signal<ChartConfiguration<'line'> | null>(null);
  acceptChart = signal<ChartConfiguration<'bar'> | null>(null);

  baseOptions: any = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: { labels: { color: '#e6e6e6', boxWidth: 10 } },
    },
    scales: {
      x: { ticks: { color: '#7b8794' }, grid: { color: '#1f242c' } },
      y: { ticks: { color: '#7b8794' }, grid: { color: '#1f242c' } },
    },
  };

  constructor(private api: ApiService) {}

  ngOnInit() {
    this.api.overviewToday().subscribe((v) => this.today.set(v));
    this.api.activeTimeToday().subscribe((v) => this.activeTime.set(v));
    this.api.costByDay(30).subscribe((s) => this.costChart.set(this.toCostChart(s)));
    this.api.tokensByDay(30).subscribe((s) => this.tokensChart.set(this.toTokensChart(s)));
    this.api.acceptByLanguage().subscribe((rows) => {
      this.accept.set(rows);
      this.acceptChart.set(this.toAcceptChart(rows));
    });
  }

  totalActive(): number {
    const t = this.activeTime();
    return t ? t.user_seconds + t.cli_seconds : 0;
  }

  fmtDuration(secs: number): string {
    if (!secs) return '0m';
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
  }

  private toCostChart(s: DailySeries): ChartConfiguration<'bar'> {
    return {
      type: 'bar',
      data: {
        labels: s.days,
        datasets: s.series.map((ser, i) => ({
          label: ser.name,
          data: ser.values,
          backgroundColor: MODEL_COLORS[i % MODEL_COLORS.length],
          stack: 'cost',
        })),
      },
      options: {
        ...this.baseOptions,
        scales: {
          x: { stacked: true, ticks: { color: '#7b8794' }, grid: { color: '#1f242c' } },
          y: { stacked: true, ticks: { color: '#7b8794' }, grid: { color: '#1f242c' } },
        },
      },
    };
  }

  private toTokensChart(s: DailySeries): ChartConfiguration<'line'> {
    return {
      type: 'line',
      data: {
        labels: s.days,
        datasets: s.series.map((ser, i) => ({
          label: ser.name,
          data: ser.values,
          borderColor: MODEL_COLORS[i % MODEL_COLORS.length],
          backgroundColor: MODEL_COLORS[i % MODEL_COLORS.length] + '33',
          tension: 0.3,
          fill: false,
        })),
      },
      options: this.baseOptions as ChartConfiguration<'line'>['options'],
    };
  }

  private toAcceptChart(rows: AcceptByLanguage[]): ChartConfiguration<'bar'> {
    const sorted = [...rows].sort((a, b) => b.accept_rate - a.accept_rate);
    return {
      type: 'bar',
      data: {
        labels: sorted.map((r) => r.language),
        datasets: [
          {
            label: 'Accept rate',
            data: sorted.map((r) => r.accept_rate),
            backgroundColor: '#facc15',
          },
        ],
      },
      options: {
        ...this.baseOptions,
        indexAxis: 'y',
        scales: {
          x: { min: 0, max: 1, ticks: { color: '#7b8794' }, grid: { color: '#1f242c' } },
          y: { ticks: { color: '#7b8794' }, grid: { color: '#1f242c' } },
        },
      } as ChartConfiguration<'bar'>['options'],
    };
  }
}
