import { ComponentFixture, TestBed } from '@angular/core/testing';
import { importProvidersFrom } from '@angular/core';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting, HttpTestingController } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';
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

  afterEach(() => http.verify());
});
