# Memory Markdown Viewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render a memory's read view as sanitized markdown with Prism-highlighted code, while keeping unparsed files and the editor as raw text and never letting a hostile memory execute script.

**Architecture:** Add ngx-markdown (which wraps `marked` and sanitizes via Angular's `DomSanitizer` by default) plus Prism.js for highlighting. The read-view `<pre>{{ body }}</pre>` becomes a branch: parsed memories render through `<markdown [data]>`; unparsed memories keep the raw `<pre>`. Rendered-HTML styling lives in the global stylesheet because ngx-markdown injects content via `innerHTML`, which Angular's emulated view encapsulation does not reach.

**Tech Stack:** Angular 21 (standalone, signals, OnPush), ngx-markdown 21.3.0, marked 18, prismjs 1.30, Vitest.

## Global Constraints

- Angular standalone components only. No NgModules. `inject()`, `ChangeDetectionStrategy.OnPush`, `@if`/`@for` (never `*ngIf`/`*ngFor`).
- Do NOT add DOMPurify or highlight.js. Sanitization is Angular's `DomSanitizer` via ngx-markdown's default.
- `[disableSanitizer]` MUST stay false (the default). Never set it true in shipped code.
- Tailwind utilities first; custom CSS only where utilities do not cover the case (the rendered-markdown element styles).
- US English everywhere. Conventional Commits (`type(scope): subject`, no emojis). TDD: failing test first.
- Target versions: `ngx-markdown@^21.3.0`, `marked@^18.0.0`, `prismjs@^1.30.0`. `marked` is a required peer; `prismjs` is an optional peer we install deliberately.

## File Structure

- Modify `web/package.json` — add the three dependencies.
- Modify `web/angular.json` — add the Prism theme CSS to `styles` and Prism core + language components to `scripts`.
- Modify `web/src/app/app.config.ts` — add `provideMarkdown()`.
- Modify `web/src/app/features/memory/memory.component.ts` — import `MarkdownComponent`.
- Modify `web/src/app/features/memory/memory.component.html` — branch the read view on `parse_ok`.
- Modify `web/src/styles.css` — global `.md-render` styles for the injected markdown HTML.
- Modify `web/src/app/features/memory/memory.component.spec.ts` — add `provideMarkdown()` to the TestBed; new tests for rendering, fallback, and the XSS gate.

---

### Task 1: Dependencies, provider wiring, and read-view markdown rendering

**Files:**
- Modify: `web/package.json`
- Modify: `web/angular.json`
- Modify: `web/src/app/app.config.ts`
- Modify: `web/src/app/features/memory/memory.component.ts`
- Modify: `web/src/app/features/memory/memory.component.html:122`
- Modify: `web/src/styles.css`
- Test: `web/src/app/features/memory/memory.component.spec.ts`

**Interfaces:**
- Consumes: `MemoryEntry.doc` fields `body: string`, `raw: string`, `parse_ok: boolean` (from `web/src/app/core/models.ts`).
- Produces: the read view renders `doc.body` as markdown for `parse_ok === true` entries via ngx-markdown's `<markdown [data]>`. Later tasks rely on this element being present for parsed entries.

- [ ] **Step 1: Install the dependencies**

Run:
```bash
cd web && npm install ngx-markdown@^21.3.0 marked@^18.0.0 prismjs@^1.30.0 --save
```
Expected: adds all three to `package.json` `dependencies`; no peer errors (katex/mermaid/clipboard/emoji-toolkit are optional peers and stay uninstalled).

- [ ] **Step 2: Register Prism assets in `angular.json`**

In the build target `options`, set `styles` and `scripts` (the `application` builder supports both). Replace the existing `"styles": ["src/styles.css"], "scripts": []`:
```json
"styles": [
  "src/styles.css",
  "node_modules/prismjs/themes/prism-tomorrow.css"
],
"scripts": [
  "node_modules/prismjs/prism.js",
  "node_modules/prismjs/components/prism-typescript.min.js",
  "node_modules/prismjs/components/prism-bash.min.js",
  "node_modules/prismjs/components/prism-json.min.js",
  "node_modules/prismjs/components/prism-rust.min.js",
  "node_modules/prismjs/components/prism-sql.min.js",
  "node_modules/prismjs/components/prism-yaml.min.js",
  "node_modules/prismjs/components/prism-powershell.min.js",
  "node_modules/prismjs/components/prism-python.min.js",
  "node_modules/prismjs/components/prism-diff.min.js"
]
```
(`prism.js` core already provides markup/css/clike/javascript; `prism-typescript` extends javascript.)

- [ ] **Step 3: Add `provideMarkdown()` to the app config**

In `web/src/app/app.config.ts`, add the import and the provider:
```typescript
import { provideMarkdown } from 'ngx-markdown';
```
Add `provideMarkdown(),` to the `providers` array (after `provideHttpClient()`).

- [ ] **Step 4: Write the failing test**

In `web/src/app/features/memory/memory.component.spec.ts`, add `provideMarkdown()` to the TestBed `providers` (import it: `import { provideMarkdown } from 'ngx-markdown';`). Then add:
```typescript
it('renders a parsed memory body as markdown, not literal text', () => {
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

  const el = fixture.nativeElement as HTMLElement;
  // Markdown was parsed into real elements, not shown as literal "# Big Heading".
  const h1 = el.querySelector('.md-render h1');
  expect(h1?.textContent).toContain('Big Heading');
  expect(el.querySelector('.md-render strong')?.textContent).toBe('bold');
  expect(el.querySelector('.md-render pre code')).toBeTruthy();
  // The raw hash must NOT appear as visible text.
  expect(el.querySelector('.md-render')?.textContent).not.toContain('# Big Heading');
});
```

- [ ] **Step 5: Run the test, verify it fails**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts -t "renders a parsed memory body as markdown"`
Expected: FAIL — `.md-render h1` is null because the read view still renders `{{ e.doc.body }}` inside a `<pre>`.

- [ ] **Step 6: Import `MarkdownComponent` in the component**

In `web/src/app/features/memory/memory.component.ts`, add to imports:
```typescript
import { MarkdownComponent } from 'ngx-markdown';
```
And add `MarkdownComponent` to the `@Component({ imports: [...] })` array (alongside `RouterLink, LucideAngularModule`).

- [ ] **Step 7: Render the read view as markdown**

In `web/src/app/features/memory/memory.component.html`, replace the read-view line (currently line 122):
```html
<pre class="whitespace-pre-wrap text-sm text-text">{{ e.doc.body }}</pre>
```
with:
```html
<div class="md-render text-sm text-text">
  <markdown [data]="e.doc.body"></markdown>
</div>
```

- [ ] **Step 8: Add global styles for the rendered markdown**

Append to `web/src/styles.css` (global, because ngx-markdown injects via `innerHTML` and component-scoped styles will not reach it). Use existing theme CSS variables/tokens the project already defines (`--border`, `--panel`, `--panel-2`, `--muted`, `--accent` — confirm the exact token names in `src/styles.css` and match them):
```css
.md-render { line-height: 1.55; }
.md-render h1 { font-size: 1.35rem; font-weight: 600; margin: 0.6em 0 0.35em; }
.md-render h2 { font-size: 1.15rem; font-weight: 600; margin: 0.6em 0 0.35em; }
.md-render h3 { font-size: 1.02rem; font-weight: 600; margin: 0.6em 0 0.35em; }
.md-render p { margin: 0.4em 0; }
.md-render ul, .md-render ol { margin: 0.4em 0; padding-left: 1.4em; }
.md-render li { margin: 0.15em 0; }
.md-render a { color: var(--accent); text-decoration: underline; }
.md-render code { font-family: ui-monospace, monospace; font-size: 0.9em; }
.md-render :not(pre) > code { background: var(--panel-2); padding: 0.1em 0.35em; border-radius: 4px; }
.md-render pre { background: var(--panel); border: 1px solid var(--border); border-radius: 8px; padding: 0.7em 0.9em; overflow-x: auto; margin: 0.5em 0; }
.md-render blockquote { border-left: 3px solid var(--border); margin: 0.5em 0; padding-left: 0.8em; color: var(--muted); }
.md-render table { border-collapse: collapse; margin: 0.5em 0; }
.md-render th, .md-render td { border: 1px solid var(--border); padding: 0.25em 0.6em; }
.md-render img { max-width: 100%; }
```

- [ ] **Step 9: Run the test, verify it passes**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts -t "renders a parsed memory body as markdown"`
Expected: PASS.

- [ ] **Step 10: Run the whole memory spec, verify no regressions**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts`
Expected: all tests PASS (existing tests now instantiate `MarkdownComponent`, which needs the `provideMarkdown()` added in Step 4 — confirm none error on missing `MarkdownService`).

- [ ] **Step 11: Commit**

```bash
git add web/package.json web/package-lock.json web/angular.json web/src/app/app.config.ts web/src/app/features/memory/memory.component.ts web/src/app/features/memory/memory.component.html web/src/styles.css web/src/app/features/memory/memory.component.spec.ts
git commit -m "feat(memory): render parsed memory bodies as markdown with Prism"
```

---

### Task 2: Plain-text fallback for unparsed memories

**Files:**
- Modify: `web/src/app/features/memory/memory.component.html`
- Test: `web/src/app/features/memory/memory.component.spec.ts`

**Interfaces:**
- Consumes: `doc.parse_ok` and `doc.body` from Task 1's read view.
- Produces: `parse_ok === false` entries render as a raw `<pre>`, not through `<markdown>`.

- [ ] **Step 1: Write the failing test**

```typescript
it('renders an unparsed memory as raw text, not markdown', () => {
  flushProjects();
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
          body: '---\nname: broken\n---\n# Not really a heading',
          raw: '---\nname: broken\n---\n# Not really a heading',
          parse_ok: false,
        },
        origin: null,
      },
    ],
  });
  fixture.detectChanges();

  const el = fixture.nativeElement as HTMLElement;
  // Unparsed files must NOT be markdown-rendered.
  expect(el.querySelector('.md-render')).toBeNull();
  // The raw text is shown verbatim in a <pre>, hashes and dashes intact.
  const pre = el.querySelector('pre');
  expect(pre?.textContent).toContain('# Not really a heading');
  expect(pre?.textContent).toContain('---');
});
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts -t "renders an unparsed memory as raw text"`
Expected: FAIL — after Task 1 every read view (including unparsed) renders through `<markdown>`, so `.md-render` is present and no plain `<pre>` exists.

- [ ] **Step 3: Branch the read view on `parse_ok`**

In `web/src/app/features/memory/memory.component.html`, replace the Task 1 read-view block:
```html
<div class="md-render text-sm text-text">
  <markdown [data]="e.doc.body"></markdown>
</div>
```
with:
```html
@if (e.doc.parse_ok) {
  <div class="md-render text-sm text-text">
    <markdown [data]="e.doc.body"></markdown>
  </div>
} @else {
  <pre class="whitespace-pre-wrap text-sm text-text">{{ e.doc.body }}</pre>
}
```

- [ ] **Step 4: Run both read-view tests, verify they pass**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts -t "memory"`
Expected: both "renders a parsed memory body as markdown" and "renders an unparsed memory as raw text" PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/memory/memory.component.html web/src/app/features/memory/memory.component.spec.ts
git commit -m "feat(memory): keep unparsed memories as raw text, not garbled markdown"
```

---

### Task 3: XSS gate — a hostile memory must not execute

**Files:**
- Test: `web/src/app/features/memory/memory.component.spec.ts`

**Interfaces:**
- Consumes: the Task 1 markdown read view and ngx-markdown's default sanitizer.
- Produces: a regression guard proving dangerous markup is stripped.

- [ ] **Step 1: Write the security test**

```typescript
it('neutralizes a hostile memory body — no script, handlers, or javascript: survive', () => {
  flushProjects();
  http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
    slug: 'D--Repos-andon',
    index: null,
    entries: [
      {
        doc: {
          file: 'evil.md',
          name: 'evil',
          description: null,
          kind: null,
          body:
            'Look here:\n\n' +
            '<img src=x onerror="window.__xss=1">\n\n' +
            '<script>window.__xss=1</script>\n\n' +
            '<a href="javascript:window.__xss=1" onclick="window.__xss=1">click</a>',
          raw: 'irrelevant',
          parse_ok: true,
        },
        origin: null,
      },
    ],
  });
  fixture.detectChanges();

  const md = (fixture.nativeElement as HTMLElement).querySelector('.md-render')!;
  expect(md.querySelector('script')).toBeNull();
  const img = md.querySelector('img');
  expect(img?.getAttribute('onerror')).toBeNull();
  const anchor = md.querySelector('a');
  expect(anchor?.getAttribute('onclick')).toBeNull();
  expect((anchor?.getAttribute('href') ?? '').toLowerCase()).not.toContain('javascript:');
});
```

- [ ] **Step 2: Run the test, verify it PASSES (sanitizer already active)**

Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts -t "neutralizes a hostile memory"`
Expected: PASS — ngx-markdown sanitizes by default. This is a guard test; it passes because Task 1 relies on the default sanitizer.

- [ ] **Step 3: Prove the test has teeth**

Temporarily add `[disableSanitizer]="true"` to the `<markdown>` element in `memory.component.html`, then rerun the test.
Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts -t "neutralizes a hostile memory"`
Expected: FAIL — with the sanitizer off, the handlers/script survive and the assertions break. This confirms the test actually detects the vulnerability.

- [ ] **Step 4: Revert the sabotage**

Remove `[disableSanitizer]="true"` from the `<markdown>` element. Rerun the test.
Run: `cd web && npx vitest run src/app/features/memory/memory.component.spec.ts -t "neutralizes a hostile memory"`
Expected: PASS again. Confirm `memory.component.html` no longer contains `disableSanitizer` anywhere:
```bash
grep -n disableSanitizer web/src/app/features/memory/memory.component.html || echo "clean"
```
Expected: `clean`.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/memory/memory.component.spec.ts
git commit -m "test(memory): assert hostile memory bodies render inert"
```

---

### Task 4: Full-suite green and live verification

**Files:** none (verification only).

- [ ] **Step 1: Run the full web suite**

Run: `cd web && npm test`
Expected: all files pass, output pristine (the pre-existing `NG8113 DatePipe` warning from `files.component.ts` is unrelated and may remain).

- [ ] **Step 2: Rebuild the SPA and launch the app**

The dev app serves the prebuilt bundle (see `docs/building.md`) — a source change is invisible until rebuild + restart.
```bash
cd web && npm run build
```
Then from the repo root, with any running installed Andon closed so :8765 is free:
```bash
cargo tauri dev
```

- [ ] **Step 3: Eyeball a real memory**

Open the Memory page. A memory with headings and a fenced code block must render with formatting and Prism-highlighted code. An unparsed memory (the `unparsed` badge) must show raw text. Confirm no console errors in the webview devtools.

- [ ] **Step 4: Final commit if any verification fixups were needed**

Only if Step 3 surfaced a fix. Otherwise the feature is complete on the branch.

---

## Self-Review

**Spec coverage:**
- ngx-markdown + provideMarkdown → Task 1 Steps 1, 3. ✓
- Angular DomSanitizer, no DOMPurify → Task 1 (default sanitizer), Task 3 (proven). ✓
- Prism.js highlighter → Task 1 Step 2. ✓
- `parse_ok === false` plain-text fallback → Task 2. ✓
- MEMORY.md index stays raw → unchanged by design; no task touches it. ✓
- Editor unchanged (raw `doc.raw` textarea) → not modified by any task; existing tests guard it (Task 1 Step 10). ✓
- XSS test → Task 3. ✓
- `[disableSanitizer]` false → Task 3 Step 4 grep guard. ✓

**Placeholder scan:** No TBD/TODO. One item needs on-the-spot confirmation: the exact CSS variable token names in `src/styles.css` (Task 1 Step 8 says to confirm and match them). This is a real lookup against the file, not a deferred decision.

**Type consistency:** `MarkdownComponent` and `provideMarkdown` both from `ngx-markdown`; `.md-render` wrapper class is consistent across Tasks 1–3 tests and the template; `doc.parse_ok`/`doc.body` match `web/src/app/core/models.ts`.
