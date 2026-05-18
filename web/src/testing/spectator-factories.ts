// Re-exports for test files.
//
// NOTE: @ngneat/spectator's createServiceFactory / createComponentFactory do not
// work with @analogjs/vitest-angular's zone setup on Angular 21 — the zone
// cleanup hook fires between Spectator's configureTestingModule call and inject,
// nulling Angular's compiler. Use bare TestBed.configureTestingModule + TestBed.inject /
// TestBed.createComponent instead. See filter.service.spec.ts and
// filter-bar.component.spec.ts for the working pattern.
export { MockComponent, MockProvider, MockDirective, MockPipe } from 'ng-mocks';
