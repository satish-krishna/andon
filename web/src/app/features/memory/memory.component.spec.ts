import { ComponentFixture, TestBed } from '@angular/core/testing';
import { importProvidersFrom } from '@angular/core';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting, HttpTestingController } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';
import { provideMarkdown } from 'ngx-markdown';
import { vi } from 'vitest';
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
        provideMarkdown(),
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

  it('does not delete when the confirm is declined', () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: null,
            kind: 'user',
            body: 'b',
            raw: 'b',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.remove('user_role.md');
    expect(confirmSpy).toHaveBeenCalled();
    http.expectNone('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete');
  });

  it('posts a delete when the confirm is accepted', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [],
    });
    fixture.detectChanges();

    fixture.componentInstance.remove('user_role.md');
    const req = http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete');
    expect(req.request.body).toEqual({ file: 'user_role.md' });
    req.flush(null);
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [],
    });
    // Delete also re-fetches the project list to refresh the switcher count.
    http
      .expectOne('http://127.0.0.1:8765/api/memory/projects')
      .flush([{ slug: 'D--Repos-andon', label: 'D:\\Repos\\andon', count: 0 }]);
  });

  it('seeds the draft from doc.raw, not doc.body, so frontmatter survives an edit', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: null,
            kind: 'user',
            body: 'The user maintains Andon.',
            raw: '---\ntype: user\n---\nThe user maintains Andon.',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const entry = fixture.componentInstance.entries()[0];
    fixture.componentInstance.startEdit(entry);

    expect(fixture.componentInstance.draft()).toBe('---\ntype: user\n---\nThe user maintains Andon.');
    expect(fixture.componentInstance.draft()).not.toBe(entry.doc.body);
  });

  it('PUTs a body whose content equals draft() verbatim', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: null,
            kind: 'user',
            body: 'old body',
            raw: 'old raw',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const entry = fixture.componentInstance.entries()[0];
    fixture.componentInstance.startEdit(entry);
    fixture.componentInstance.draft.set('edited content with\nnewlines and --- frontmatter markers');
    fixture.componentInstance.saveEdit('user_role.md');

    const req = http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/file');
    expect(req.request.method).toBe('PUT');
    expect(req.request.body).toEqual({
      file: 'user_role.md',
      content: 'edited content with\nnewlines and --- frontmatter markers',
    });
    req.flush(null);
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon')
      .flush({ slug: 'D--Repos-andon', index: null, entries: [] });
  });

  it('round-trips an unparsed entry (parse_ok: false) identically via raw', () => {
    flushProjects();
    const rawText = 'garbled: [unterminated\nSome content that failed frontmatter parsing.';
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'broken.md',
            name: null,
            description: null,
            kind: null,
            body: 'Some content that failed frontmatter parsing.',
            raw: rawText,
            parse_ok: false,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const entry = fixture.componentInstance.entries()[0];
    expect(entry.doc.parse_ok).toBe(false);
    fixture.componentInstance.startEdit(entry);
    expect(fixture.componentInstance.draft()).toBe(rawText);

    fixture.componentInstance.saveEdit('broken.md');
    const req = http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/file');
    expect(req.request.body).toEqual({ file: 'broken.md', content: rawText });
    req.flush(null);
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon')
      .flush({ slug: 'D--Repos-andon', index: null, entries: [] });
  });

  it('cancelEdit clears editing and draft', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'user_role.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const entry = fixture.componentInstance.entries()[0];
    fixture.componentInstance.startEdit(entry);
    expect(fixture.componentInstance.editing()).toBe('user_role.md');

    fixture.componentInstance.cancelEdit();
    expect(fixture.componentInstance.editing()).toBeNull();
    expect(fixture.componentInstance.draft()).toBe('');
  });

  it('clears edit state on project switch and never saves a stale cross-project draft', () => {
    fixture.detectChanges();
    http.expectOne('http://127.0.0.1:8765/api/memory/projects').flush([
      { slug: 'proj-a', label: 'Project A', count: 1 },
      { slug: 'proj-b', label: 'Project B', count: 1 },
    ]);
    fixture.detectChanges();

    // proj-a is auto-selected by ngOnInit.
    http.expectOne('http://127.0.0.1:8765/api/memory/proj-a').flush({
      slug: 'proj-a',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: null,
            description: null,
            kind: null,
            body: 'A body',
            raw: 'A original',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const entryA = fixture.componentInstance.entries()[0];
    fixture.componentInstance.startEdit(entryA);
    fixture.componentInstance.draft.set('A modified — must never land in project B');
    expect(fixture.componentInstance.editing()).toBe('user_role.md');

    // Switch to project B, which also has a user_role.md.
    fixture.componentInstance.select('proj-b');
    fixture.detectChanges();
    http.expectOne('http://127.0.0.1:8765/api/memory/proj-b').flush({
      slug: 'proj-b',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: null,
            description: null,
            kind: null,
            body: 'B body',
            raw: 'B original',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    // The project switch must clear the stale edit state.
    expect(fixture.componentInstance.editing()).toBeNull();
    expect(fixture.componentInstance.draft()).toBe('');

    // Even if a stale click still reaches saveEdit, it must not PUT project B with project A's content.
    fixture.componentInstance.saveEdit('user_role.md');
    http.expectNone('http://127.0.0.1:8765/api/memory/proj-b/file');
  });

  it('saveEdit refuses to write if the edit was started under a different project (defense in depth)', () => {
    flushProjects();
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon')
      .flush({ slug: 'D--Repos-andon', index: null, entries: [] });
    fixture.detectChanges();

    // Simulate a stale in-flight edit whose editingSlug no longer matches the
    // currently selected project, bypassing the state-clearing path entirely.
    fixture.componentInstance.editingSlug.set('some-other-project');
    fixture.componentInstance.draft.set('stale content');

    fixture.componentInstance.saveEdit('user_role.md');
    http.expectNone('http://127.0.0.1:8765/api/memory/D--Repos-andon/file');
  });

  it('renders an error and keeps the editor open when save fails', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'user_role.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const entry = fixture.componentInstance.entries()[0];
    fixture.componentInstance.startEdit(entry);
    fixture.componentInstance.saveEdit('user_role.md');

    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/file')
      .flush({ error: 'save rejected' }, { status: 400, statusText: 'Bad Request' });
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain("Couldn't save");
    // The editor stays open — nothing was silently lost.
    expect(fixture.componentInstance.editing()).toBe('user_role.md');
  });

  it('renders an error when delete fails', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'user_role.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.remove('user_role.md');
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete')
      .flush({ error: 'delete rejected' }, { status: 400, statusText: 'Bad Request' });
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain("Couldn't delete");
  });

  it('confirms before discarding a dirty draft when switching edit targets', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'a.md', name: null, description: null, kind: null, body: 'a', raw: 'a', parse_ok: true },
          origin: null,
        },
        {
          doc: { file: 'b.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const [entryA, entryB] = fixture.componentInstance.entries();
    fixture.componentInstance.startEdit(entryA);
    fixture.componentInstance.draft.set('dirty change');

    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    confirmSpy.mockClear();
    fixture.componentInstance.startEdit(entryB);

    expect(confirmSpy).toHaveBeenCalled();
    // Confirm declined — original edit target is preserved.
    expect(fixture.componentInstance.editing()).toBe('a.md');
    expect(fixture.componentInstance.draft()).toBe('dirty change');
  });

  it('does not confirm when switching edit targets with an untouched draft', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'a.md', name: null, description: null, kind: null, body: 'a', raw: 'a', parse_ok: true },
          origin: null,
        },
        {
          doc: { file: 'b.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const [entryA, entryB] = fixture.componentInstance.entries();
    fixture.componentInstance.startEdit(entryA);
    // No changes made to the draft.

    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    confirmSpy.mockClear();
    fixture.componentInstance.startEdit(entryB);

    expect(confirmSpy).not.toHaveBeenCalled();
    expect(fixture.componentInstance.editing()).toBe('b.md');
  });

  it('refresh mid-edit preserves the draft and editing (does not vaporize unsaved keystrokes)', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'user_role.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const entry = fixture.componentInstance.entries()[0];
    fixture.componentInstance.startEdit(entry);
    fixture.componentInstance.draft.set('unsaved keystrokes');

    // The always-visible manual Refresh button, mid-edit — must not clear the draft.
    fixture.componentInstance.refresh();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'user_role.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    expect(fixture.componentInstance.editing()).toBe('user_role.md');
    expect(fixture.componentInstance.draft()).toBe('unsaved keystrokes');
  });

  it('drops a late memoryList response for a project the user has already left', () => {
    fixture.detectChanges();
    http.expectOne('http://127.0.0.1:8765/api/memory/projects').flush([
      { slug: 'proj-a', label: 'Project A', count: 1 },
      { slug: 'proj-b', label: 'Project B', count: 1 },
    ]);
    fixture.detectChanges();

    // proj-a is auto-selected by ngOnInit; its GET is now in flight.
    const reqA = http.expectOne('http://127.0.0.1:8765/api/memory/proj-a');

    // Switch to proj-b before A resolves.
    fixture.componentInstance.select('proj-b');
    fixture.detectChanges();
    const reqB = http.expectOne('http://127.0.0.1:8765/api/memory/proj-b');

    reqB.flush({
      slug: 'proj-b',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: null,
            description: null,
            kind: null,
            body: 'B body',
            raw: 'B raw',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    // A finally resolves, after B — same-named file, different project. Must be dropped.
    reqA.flush({
      slug: 'proj-a',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: null,
            description: null,
            kind: null,
            body: 'A body',
            raw: 'A raw',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    expect(fixture.componentInstance.slug()).toBe('proj-b');
    expect(fixture.componentInstance.entries()[0].doc.body).toBe('B body');
    expect(fixture.componentInstance.loading()).toBe(false);
  });

  it('does not let a dropped stale response clear loading owned by a newer in-flight request', () => {
    fixture.detectChanges();
    http.expectOne('http://127.0.0.1:8765/api/memory/projects').flush([
      { slug: 'proj-a', label: 'Project A', count: 1 },
      { slug: 'proj-b', label: 'Project B', count: 1 },
    ]);
    fixture.detectChanges();

    const reqA = http.expectOne('http://127.0.0.1:8765/api/memory/proj-a');

    fixture.componentInstance.select('proj-b');
    fixture.detectChanges();
    const reqB = http.expectOne('http://127.0.0.1:8765/api/memory/proj-b');

    // B's request is still in flight when A's stale response resolves.
    expect(fixture.componentInstance.loading()).toBe(true);
    reqA.flush({ slug: 'proj-a', index: null, entries: [] });
    fixture.detectChanges();

    // A's dropped response must not have cleared loading — B's request still owns it.
    expect(fixture.componentInstance.loading()).toBe(true);

    reqB.flush({ slug: 'proj-b', index: null, entries: [] });
    fixture.detectChanges();
    expect(fixture.componentInstance.loading()).toBe(false);
  });

  it('clears the history cache when the project changes', () => {
    fixture.detectChanges();
    http.expectOne('http://127.0.0.1:8765/api/memory/projects').flush([
      { slug: 'proj-a', label: 'Project A', count: 1 },
      { slug: 'proj-b', label: 'Project B', count: 1 },
    ]);
    fixture.detectChanges();

    // proj-a is auto-selected by ngOnInit.
    http.expectOne('http://127.0.0.1:8765/api/memory/proj-a').flush({
      slug: 'proj-a',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: null,
            description: null,
            kind: null,
            body: 'A body',
            raw: 'A raw',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    // Fetch history for a file in proj-a.
    fixture.componentInstance.toggleHistory('user_role.md');
    http
      .expectOne(
        (r) =>
          r.url === 'http://127.0.0.1:8765/api/memory/proj-a/provenance' &&
          r.params.get('file') === 'user_role.md',
      )
      .flush([{ session_id: 'sess-a', action: 'create', ts: 1 }]);
    fixture.detectChanges();

    // Cache should be populated with proj-a's history.
    expect(fixture.componentInstance.history()['user_role.md']).toBeDefined();
    expect(fixture.componentInstance.history()['user_role.md'][0].session_id).toBe('sess-a');

    // Switch to proj-b, which also has user_role.md.
    fixture.componentInstance.select('proj-b');
    fixture.detectChanges();
    http.expectOne('http://127.0.0.1:8765/api/memory/proj-b').flush({
      slug: 'proj-b',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: null,
            description: null,
            kind: null,
            body: 'B body',
            raw: 'B raw',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    // Cache and openHistory should be cleared on project switch.
    expect(fixture.componentInstance.history()).toEqual({});
    expect(fixture.componentInstance.openHistory()).toBeNull();
  });

  it('shows human edits as andon-user rather than a session link', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: null,
            kind: 'user',
            body: 'b',
            raw: 'b',
            parse_ok: true,
          },
          origin: { session_id: 'andon-user', action: 'edit', ts: 2 },
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.toggleHistory('user_role.md');
    // expectOne(string) matches req.url, which excludes query params — match on a
    // predicate so the `file` param is actually asserted.
    http
      .expectOne(
        (r) =>
          r.url === 'http://127.0.0.1:8765/api/memory/D--Repos-andon/provenance' &&
          r.params.get('file') === 'user_role.md',
      )
      .flush([
        { session_id: 'andon-user', action: 'edit', ts: 2 },
        { session_id: 'sess-1', action: 'create', ts: 1 },
      ]);
    fixture.detectChanges();

    const el = fixture.nativeElement as HTMLElement;
    expect(el.textContent).toContain('You, in Andon');
    expect(el.querySelector('a[href="/sessions/andon-user"]')).toBeNull();
    expect(el.querySelector('a[href="/sessions/sess-1"]')).toBeTruthy();
  });

  it('renders two touches sharing the same ts without an NG0955 duplicate-key warning', () => {
    // Touch has no row id and two touches to the same file can legitimately share a
    // ts (see provenance.rs's ts-tie tests). `track t.ts` gives Angular's @for a
    // duplicate key; Angular's live-collection reconciler flags that with an
    // NG0955 dev-mode console.warn precisely because it is undefined behavior.
    // Angular's duplicate-key check only runs while reconciling an ALREADY-mounted
    // @for against a live collection (not on first creation from empty), so this
    // opens the disclosure with one touch first, then updates the history cache
    // in place — mirroring `toggleHistory`'s deliberate re-fetch-while-open
    // behavior — to a two-touch list that ties on ts, without unmounting the
    // @if block in between.
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: null,
            kind: 'user',
            body: 'b',
            raw: 'b',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.toggleHistory('user_role.md');
    const req = http.expectOne(
      (r) =>
        r.url === 'http://127.0.0.1:8765/api/memory/D--Repos-andon/provenance' &&
        r.params.get('file') === 'user_role.md',
    );
    req.flush([{ session_id: 'sess-old', action: 'create', ts: 50 }]);
    fixture.detectChanges();

    // Simulate the ledger gaining a tied-ts row while the disclosure stays open,
    // by updating the underlying cache signal directly (same mechanism
    // `toggleHistory` itself uses on a re-fetch).
    fixture.componentInstance.history.set({
      'user_role.md': [
        { session_id: 'sess-a', action: 'create', ts: 100 },
        { session_id: 'sess-b', action: 'update', ts: 100 },
      ],
    });

    expect(() => fixture.detectChanges()).not.toThrow();

    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('a[href="/sessions/sess-a"]')).toBeTruthy();
    expect(el.querySelector('a[href="/sessions/sess-b"]')).toBeTruthy();
    const ng0955 = warnSpy.mock.calls.some((c) => String(c[0]).includes('NG0955'));
    expect(ng0955).toBe(false);
  });

  it('clears the cached history for a file after a successful delete', () => {
    // Repro: open history on a file, delete it. A later reappearance (the model
    // reinstating the memory) must not render the pre-delete history from a stale
    // cache — the delete + re-create IS the churn signal this feature exists to
    // surface, and a stale cache would hide it.
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'user_role.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: { session_id: 'sess-x', action: 'create', ts: 1 },
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.toggleHistory('user_role.md');
    http
      .expectOne(
        (r) =>
          r.url === 'http://127.0.0.1:8765/api/memory/D--Repos-andon/provenance' &&
          r.params.get('file') === 'user_role.md',
      )
      .flush([{ session_id: 'sess-x', action: 'create', ts: 1 }]);
    fixture.detectChanges();
    expect(fixture.componentInstance.history()['user_role.md']).toBeDefined();
    expect(fixture.componentInstance.openHistory()).toBe('user_role.md');

    fixture.componentInstance.remove('user_role.md');
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete').flush(null);
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon')
      .flush({ slug: 'D--Repos-andon', index: null, entries: [] });
    http
      .expectOne('http://127.0.0.1:8765/api/memory/projects')
      .flush([{ slug: 'D--Repos-andon', label: 'D:\\Repos\\andon', count: 0 }]);
    fixture.detectChanges();

    expect(fixture.componentInstance.history()['user_role.md']).toBeUndefined();
    expect(fixture.componentInstance.openHistory()).toBeNull();
  });

  it('closes the open history disclosure and drops its stale cache after a successful save', () => {
    // Milder same-page version: Save must not leave the still-open history list
    // showing rows that predate the edit just made.
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'user_role.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.toggleHistory('user_role.md');
    http
      .expectOne(
        (r) =>
          r.url === 'http://127.0.0.1:8765/api/memory/D--Repos-andon/provenance' &&
          r.params.get('file') === 'user_role.md',
      )
      .flush([{ session_id: 'sess-old', action: 'create', ts: 1 }]);
    fixture.detectChanges();
    expect(fixture.componentInstance.openHistory()).toBe('user_role.md');

    const entry = fixture.componentInstance.entries()[0];
    fixture.componentInstance.startEdit(entry);
    fixture.componentInstance.draft.set('edited content');
    fixture.componentInstance.saveEdit('user_role.md');

    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/file').flush(null);
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon')
      .flush({ slug: 'D--Repos-andon', index: null, entries: [] });
    fixture.detectChanges();

    expect(fixture.componentInstance.history()['user_role.md']).toBeUndefined();
    expect(fixture.componentInstance.openHistory()).toBeNull();
  });

  it('re-fetches the project list after a successful delete so the switcher count is not stale', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    flushProjects(2);
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'a.md', name: null, description: null, kind: null, body: 'a', raw: 'a', parse_ok: true },
          origin: null,
        },
        {
          doc: { file: 'b.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();
    expect(fixture.componentInstance.projects()[0].count).toBe(2);

    fixture.componentInstance.remove('a.md');
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete').flush(null);
    // refresh() re-reads the current project's entries...
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'b.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    // ...and the switcher's project list is re-fetched so its count reflects the delete.
    http
      .expectOne('http://127.0.0.1:8765/api/memory/projects')
      .flush([{ slug: 'D--Repos-andon', label: 'D:\\Repos\\andon', count: 1 }]);
    fixture.detectChanges();

    expect(fixture.componentInstance.projects()[0].count).toBe(1);
  });

  it('preserves the selected project and its entries when the post-delete projects re-fetch lands', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    flushProjects(2);
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'a.md', name: null, description: null, kind: null, body: 'a', raw: 'a', parse_ok: true },
          origin: null,
        },
        {
          doc: { file: 'b.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.remove('a.md');
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete').flush(null);
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'b.md', name: null, description: null, kind: null, body: 'b', raw: 'b', parse_ok: true },
          origin: null,
        },
      ],
    });
    // The re-fetched list sorts a DIFFERENT project first — proving slug is not
    // re-derived from projects[0] when the global list updates.
    http.expectOne('http://127.0.0.1:8765/api/memory/projects').flush([
      { slug: 'A--other', label: 'Other', count: 3 },
      { slug: 'D--Repos-andon', label: 'D:\\Repos\\andon', count: 1 },
    ]);
    fixture.detectChanges();

    expect(fixture.componentInstance.slug()).toBe('D--Repos-andon');
    expect(fixture.componentInstance.entries().map((e) => e.doc.file)).toEqual(['b.md']);
  });

  it('leaves the switcher intact and raises no load error when the post-delete projects re-fetch fails', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    flushProjects(2);
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: { file: 'a.md', name: null, description: null, kind: null, body: 'a', raw: 'a', parse_ok: true },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.remove('a.md');
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete').flush(null);
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon')
      .flush({ slug: 'D--Repos-andon', index: null, entries: [] });
    http
      .expectOne('http://127.0.0.1:8765/api/memory/projects')
      .flush('', { status: 500, statusText: 'Server Error' });
    fixture.detectChanges();

    // The delete itself succeeded — a failed count refresh must not blank the
    // switcher or masquerade as a page-level load error.
    expect(fixture.componentInstance.projects().length).toBe(1);
    expect(fixture.componentInstance.loadError()).toBe(false);
  });

  it('renders a parsed memory body as markdown, not literal text', async () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'guide.md',
            name: 'guide',
            description: null,
            kind: 'reference',
            body: '# Big Heading\n\nSome **bold** text.\n\n```ts\nconst x = 1;\n```',
            raw: '# Big Heading\n\nSome **bold** text.',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();
    await fixture.whenStable();

    const el = fixture.nativeElement as HTMLElement;
    // Markdown was parsed into real elements, not shown as literal "# Big Heading".
    const h1 = el.querySelector('.md-render h1');
    expect(h1?.textContent).toContain('Big Heading');
    expect(el.querySelector('.md-render strong')?.textContent).toBe('bold');
    expect(el.querySelector('.md-render pre code')).toBeTruthy();
    // The raw hash must NOT appear as visible text.
    expect(el.querySelector('.md-render')?.textContent).not.toContain('# Big Heading');
  });

  afterEach(() => http.verify());
});
