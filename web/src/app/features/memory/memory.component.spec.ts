import { ComponentFixture, TestBed } from '@angular/core/testing';
import { importProvidersFrom } from '@angular/core';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting, HttpTestingController } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';
import { APP_ICONS } from '../../core/icons';
import { MemoryComponent } from './memory.component';

describe('MemoryComponent', () => {
  let fixture: ComponentFixture<MemoryComponent>;
  let http: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [MemoryComponent],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
      ],
    }).compileComponents();
    fixture = TestBed.createComponent(MemoryComponent);
    http = TestBed.inject(HttpTestingController);
  });

  function flushProjects(count = 1) {
    fixture.detectChanges();
    http
      .expectOne('http://127.0.0.1:8765/api/memory/projects')
      .flush([{ slug: 'D--Repos-andon', label: 'D:\\Repos\\andon', count }]);
    fixture.detectChanges();
  }

  it('renders an empty state when the project has no memories', () => {
    flushProjects(0);
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon')
      .flush({ slug: 'D--Repos-andon', index: null, entries: [] });
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('No memories');
  });

  it('labels a memory with no provenance as origin unknown', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: 'who the user is',
            kind: 'user',
            body: 'The user maintains Andon.',
            raw: 'The user maintains Andon.',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('Origin unknown');
    expect(text).toContain('user-role');
  });

  it('shows the MEMORY.md index when the project has one', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: '- [Role](user_role.md) — who they are\n',
      entries: [],
    });
    fixture.detectChanges();

    const el = fixture.nativeElement as HTMLElement;
    expect(el.textContent).toContain('MEMORY.md');
    expect(el.textContent).not.toContain('who they are');

    fixture.componentInstance.toggleIndex();
    fixture.detectChanges();
    expect((fixture.nativeElement as HTMLElement).textContent).toContain('who they are');
  });

  it('links a memory with provenance to its last-touching session', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: 'who the user is',
            kind: 'user',
            body: 'body',
            raw: 'body',
            parse_ok: true,
          },
          origin: { session_id: 'sess-9', action: 'update', ts: 1 },
        },
      ],
    });
    fixture.detectChanges();

    const link = (fixture.nativeElement as HTMLElement).querySelector('a[href="/sessions/sess-9"]');
    expect(link).toBeTruthy();
  });

  it('does not claim any empty state before the first load resolves', () => {
    fixture.detectChanges();

    // Nothing has come back from the API yet — the page must not assert "empty"
    // (per-project or machine-wide) before it has actually checked.
    const textBeforeFlush = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(textBeforeFlush).not.toContain('No memories');
    expect(textBeforeFlush).not.toContain('No project');

    http.expectOne('http://127.0.0.1:8765/api/memory/projects').flush([]);
    fixture.detectChanges();

    const textAfterFlush = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(textAfterFlush).toContain('No project on this machine has any memories yet');
  });

  it('shows a load error, not the empty state, when the project list fails to load', () => {
    fixture.detectChanges();
    http
      .expectOne('http://127.0.0.1:8765/api/memory/projects')
      .flush('', { status: 500, statusText: 'Server Error' });
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain("Couldn't load memories");
    expect(text).not.toContain('No memories');
    expect(text).not.toContain('No project');
  });

  it('shows a load error, not the empty state, when the memory list fails to load', () => {
    flushProjects();
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon')
      .flush('', { status: 500, statusText: 'Server Error' });
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain("Couldn't load memories");
    expect(text).not.toContain('No memories');
  });

  it('shows distinct copy when no project on the machine has memories yet', () => {
    fixture.detectChanges();
    http.expectOne('http://127.0.0.1:8765/api/memory/projects').flush([]);
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('No project on this machine has any memories yet');
    expect(text).not.toContain('No memories for this project yet');
  });

  afterEach(() => http.verify());
});
