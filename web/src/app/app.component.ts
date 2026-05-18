import { Component, inject, OnInit, signal } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';
import { ApiService } from './core/api.service';

@Component({
  selector: 'app-root',
  imports: [RouterOutlet, RouterLink, RouterLinkActive, LucideAngularModule],
  templateUrl: './app.component.html',
  styleUrl: './app.component.css',
})
export class AppComponent implements OnInit {
  private api = inject(ApiService);
  readonly version = signal<string | null>(null);

  ngOnInit(): void {
    this.api.health().subscribe({ next: h => this.version.set(h.version) });
  }
}
