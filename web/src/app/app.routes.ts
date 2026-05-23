import { Routes } from '@angular/router';

export const routes: Routes = [
  { path: '', pathMatch: 'full', redirectTo: 'overview' },
  {
    path: 'overview',
    loadComponent: () =>
      import('./features/overview/overview.component').then((m) => m.OverviewComponent),
  },
  {
    path: 'efficiency',
    loadComponent: () =>
      import('./features/efficiency/efficiency.component').then((m) => m.EfficiencyComponent),
  },
  {
    path: 'sessions',
    loadComponent: () =>
      import('./features/sessions/sessions.component').then((m) => m.SessionsComponent),
  },
  {
    path: 'sessions/:id',
    loadComponent: () =>
      import('./features/sessions/session-detail.component').then(
        (m) => m.SessionDetailComponent,
      ),
  },
  {
    path: 'files',
    loadComponent: () =>
      import('./features/files/files.component').then((m) => m.FilesComponent),
  },
  {
    path: 'behaviour',
    loadComponent: () =>
      import('./features/behaviour/behaviour.component').then((m) => m.BehaviourComponent),
  },
  {
    path: 'settings',
    loadComponent: () =>
      import('./features/settings/settings.component').then((m) => m.SettingsComponent),
  },
  {
    path: 'diagnostics',
    loadComponent: () =>
      import('./features/diagnostics/diagnostics.component').then(
        (m) => m.DiagnosticsComponent,
      ),
  },
];
