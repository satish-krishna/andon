import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting, HttpTestingController } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';
import { MemoryComponent } from './memory.component';

describe('MemoryComponent', () => {
  let fixture: ComponentFixture<MemoryComponent>;
  let http: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [MemoryComponent],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideRouter([])],
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

  afterEach(() => http.verify());
});
