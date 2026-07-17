import { ApplicationConfig, importProvidersFrom, provideZoneChangeDetection } from '@angular/core';
import { provideRouter, withHashLocation } from '@angular/router';
import { provideHttpClient } from '@angular/common/http';
import { Chart, registerables } from 'chart.js';
import { LucideAngularModule } from 'lucide-angular';
import { provideMarkdown } from 'ngx-markdown';

import { routes } from './app.routes';
import { APP_ICONS } from './core/icons';

Chart.register(...registerables);

export const appConfig: ApplicationConfig = {
  providers: [
    provideZoneChangeDetection({ eventCoalescing: true }),
    provideRouter(routes, withHashLocation()),
    provideHttpClient(),
    provideMarkdown(),
    importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
  ],
};
