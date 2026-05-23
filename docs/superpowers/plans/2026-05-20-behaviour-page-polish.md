# Behaviour page polish — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle the Behaviour page to the app's design system, fix slash-command detection (real records store `message.content` as a JSON string the deserialiser drops), and add an "Experimental" banner.

**Architecture:** Three independent changes. (1) A custom serde deserialiser on `Message.content` accepts a JSON string as well as an array. (2) Regression tests — a reducer unit test and a privacy proptest variant — lock the fix in. (3) `behaviour.component.html` is rewritten against the shared `.crumb` / `.panel` design system, with a static Experimental banner.

**Tech Stack:** Rust 1.95 (serde, serde_json, proptest) · Angular 21 (standalone components, signals, Tailwind 4).

**Spec:** [`docs/superpowers/specs/2026-05-20-behaviour-page-polish-design.md`](../specs/2026-05-20-behaviour-page-polish-design.md)

**Branch:** `feature/jsonl-ingest` (current branch; this work joins PR #9).

> **Deviation from spec:** the spec calls the restyle "template-only." Task 3 also edits `behaviour.component.ts` — the template needs `LucideAngularModule` for `<lucide-icon>`, and the bar visualisation needs `computed` max values. Both are unavoidable; the plan makes them explicit.

---

## File structure

### Modify (Rust)
- `src-tauri/src/jsonl/record.rs` — custom `deserialize_content` fn + `#[serde(deserialize_with)]` on `Message.content`; two unit tests.
- `src-tauri/src/jsonl/reducer.rs` — one unit test (no logic change).
- `src-tauri/tests/jsonl_privacy.rs` — one new proptest.

### Modify (Angular)
- `web/src/app/features/behaviour/behaviour.component.ts` — add `LucideAngularModule` import + two `computed` maxes.
- `web/src/app/features/behaviour/behaviour.component.html` — full restyle + Experimental banner.

### Not touched
- API routes, DTOs, the reducer's detection logic, the DB schema, other pages.

---

## Task 1: `record.rs` — accept string-form `message.content`

**Files:**
- Modify: `src-tauri/src/jsonl/record.rs`.

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/jsonl/record.rs`, append to the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn parses_string_content_as_single_text_block() {
    let line = r#"{"type":"user","sessionId":"s1","message":{"role":"user","content":"<command-name>/review</command-name>"}}"#;
    let r = parse_line(line).expect("parse");
    let msg = r.message.expect("message");
    assert_eq!(msg.content.len(), 1, "string content -> one block");
    match &msg.content[0] {
        ContentBlock::Text { text } => {
            assert_eq!(text.as_deref(), Some("<command-name>/review</command-name>"));
        }
        _ => panic!("expected a Text block"),
    }
}

#[test]
fn missing_content_is_empty_vec() {
    let line = r#"{"type":"user","sessionId":"s1","message":{"role":"user"}}"#;
    let msg = parse_line(line).expect("parse").message.expect("message");
    assert!(msg.content.is_empty(), "absent content -> empty vec");
}
```

(Array-form content is already covered by the existing `parses_user_record` and `parses_assistant_with_tool_use` tests — no new test needed for it.)

- [ ] **Step 2: Run to verify the new string test fails**

```powershell
cd src-tauri; cargo test --features test-support --lib jsonl::record
```

Expected: `parses_string_content_as_single_text_block` FAILS (`string content -> one block`: gets 0, not 1 — serde can't coerce a string into `Vec<ContentBlock>`, so `#[serde(default)]` yields an empty vec). `missing_content_is_empty_vec` passes already.

- [ ] **Step 3: Add the custom deserialiser**

In `src-tauri/src/jsonl/record.rs`, change the `content` field of `Message`:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub role: Option<String>,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    #[serde(default, deserialize_with = "deserialize_content")]
    pub content: Vec<ContentBlock>,
}
```

Then add this free function in the same file, after the `Message` struct:

```rust
/// Claude Code's per-session JSONL stores `message.content` as either a JSON
/// array of content blocks (assistant turns, tool results) or a plain JSON
/// string (simple user turns, including slash-command invocations). Serde's
/// derived impl only handles the array form, so string content was silently
/// dropped. This deserialiser accepts both: a string becomes a single
/// `Text` block; anything else (absent, null, unexpected shape) becomes an
/// empty vec, consistent with the lenient never-abort-a-line philosophy.
fn deserialize_content<'de, D>(deserializer: D) -> Result<Vec<ContentBlock>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(Value::String(s)) => vec![ContentBlock::Text { text: Some(s) }],
        Some(arr @ Value::Array(_)) => serde_json::from_value(arr).unwrap_or_default(),
        _ => vec![],
    })
}
```

`Value` and `serde::Deserialize` are already imported at the top of the file (`use serde::Deserialize;`, `use serde_json::Value;`).

- [ ] **Step 4: Run to verify all `jsonl::record` tests pass**

```powershell
cd src-tauri; cargo test --features test-support --lib jsonl::record
```

Expected: all PASS — the 5 existing record tests plus the 2 new ones (7 total).

- [ ] **Step 5: Clippy**

```powershell
cd src-tauri; cargo clippy --features test-support --all-targets -- -D warnings 2>&1 | Select-String "record.rs"
```

Expected: no clippy output mentioning `record.rs` (pre-existing errors in other files are out of scope).

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/jsonl/record.rs
git commit -m "fix(jsonl): accept string-form message.content (slash commands were dropped)"
```

---

## Task 2: Regression tests — reducer + privacy proptest

**Files:**
- Modify: `src-tauri/src/jsonl/reducer.rs` (test only).
- Modify: `src-tauri/tests/jsonl_privacy.rs`.

These tests pass as soon as they are written — Task 1 fixed the underlying bug. They are regression coverage: a reducer-level proof that string-form slash commands are now detected, and a privacy proof that string-form user content still cannot leak through the reducer.

- [ ] **Step 1: Add the reducer unit test**

In `src-tauri/src/jsonl/reducer.rs`, append to the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn string_content_user_record_emits_slash_command() {
    let mut r = Reducer::new();
    let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":"<command-name>/review</command-name><command-args>PR 42</command-args>"}}"#;
    let out = r.reduce(&parse_line(line).unwrap());
    let sc = out
        .iter()
        .find_map(|e| match e {
            DerivedEvent::SlashCommand { name, arg_count, .. } => {
                Some((name.clone(), *arg_count))
            }
            _ => None,
        })
        .expect("slash command emitted from string-form content");
    assert_eq!(sc, ("review".to_string(), 2));
}
```

- [ ] **Step 2: Add the privacy proptest variant**

In `src-tauri/tests/jsonl_privacy.rs`, add a third test inside the `proptest! { ... }` block, after `assistant_text_never_leaks`:

```rust
    #[test]
    fn user_string_content_text_never_leaks(prompt in "[A-Za-z0-9 _.,;:!?/-]{20,200}") {
        // Same guarantee as `user_text_never_leaks`, but the prompt is the
        // raw string value of `message.content` rather than an array text block.
        let rec_json = json!({
            "type": "user",
            "sessionId": "s1",
            "timestamp": "2026-05-19T10:00:00.000Z",
            "message": { "role": "user", "content": prompt }
        });
        let rec: JsonlRecord = serde_json::from_value(rec_json).unwrap();
        let mut r = Reducer::new();
        let out = r.reduce(&rec);
        let d = dump(&out);
        prop_assert!(!d.contains(&prompt), "reducer leaked string-content user text: {prompt:?}");
    }
```

- [ ] **Step 3: Run both test files**

```powershell
cd src-tauri; cargo test --features test-support --lib jsonl::reducer
cd src-tauri; cargo test --features test-support --test jsonl_privacy
```

Expected: `jsonl::reducer` — all PASS including the new `string_content_user_record_emits_slash_command`. `jsonl_privacy` — 3 PASS (`user_text_never_leaks`, `assistant_text_never_leaks`, `user_string_content_text_never_leaks`), 256 cases each.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/jsonl/reducer.rs src-tauri/tests/jsonl_privacy.rs
git commit -m "test(jsonl): cover string-form content for slash commands and privacy boundary"
```

---

## Task 3: Restyle the Behaviour page + Experimental banner

**Files:**
- Modify: `web/src/app/features/behaviour/behaviour.component.ts`.
- Modify: `web/src/app/features/behaviour/behaviour.component.html`.

- [ ] **Step 1: Update the component TypeScript**

Replace the entire contents of `web/src/app/features/behaviour/behaviour.component.ts` with:

```ts
import { CommonModule, DecimalPipe } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService } from '../../core/api.service';
import {
  ModelMixResponse,
  SlashCommandEntry,
  SubAgentEntry,
} from '../../core/models';

@Component({
  selector: 'app-behaviour',
  standalone: true,
  imports: [CommonModule, DecimalPipe, LucideAngularModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './behaviour.component.html',
})
export class BehaviourComponent {
  private readonly api = inject(ApiService);

  readonly modelMix = signal<ModelMixResponse | null>(null);
  readonly slash = signal<SlashCommandEntry[]>([]);
  readonly subs = signal<SubAgentEntry[]>([]);

  // Bar denominators. `by_model` is sorted by invocations, not sessions, so
  // each bar needs its own max. `Math.max(1, ...)` guards against an empty
  // array (-> 1) and division by zero.
  readonly invocationsMax = computed(() =>
    Math.max(1, ...(this.modelMix()?.by_model ?? []).map((m) => m.invocations)),
  );
  readonly sessionsMax = computed(() =>
    Math.max(1, ...(this.modelMix()?.by_model ?? []).map((m) => m.sessions)),
  );

  constructor() {
    this.api.modelMix().subscribe((v) => this.modelMix.set(v));
    this.api.slashCommands().subscribe((v) => this.slash.set(v));
    this.api.subagents().subscribe((v) => this.subs.set(v));
  }
}
```

- [ ] **Step 2: Replace the template**

Replace the entire contents of `web/src/app/features/behaviour/behaviour.component.html` with:

```html
<div class="crumb">
  <span class="flex items-center gap-1.5">
    <lucide-icon name="brain" class="w-3.5 h-3.5"></lucide-icon>Behaviour
  </span>
</div>

<div class="mx-6 mt-4 border border-warn/40 bg-warn/5 rounded-md px-4 py-2.5 flex items-center gap-2.5">
  <lucide-icon name="flask-conical" class="w-4 h-4 text-warn shrink-0"></lucide-icon>
  <div class="text-xs">
    <span class="text-warn font-medium">Experimental.</span>
    <span class="text-muted">Behavioural views are derived from JSONL transcripts and may be
    incomplete or change between releases.</span>
  </div>
</div>

<div class="px-6 py-5 flex flex-col gap-4">

  <!-- MODEL MIX -->
  <section class="panel">
    <div class="panel-title">Model mix</div>
    <div class="panel-body">
      @if (modelMix(); as mm) {
        @if (mm.by_model.length === 0) {
          <div class="text-muted text-xs font-mono py-4">No model data yet.</div>
        } @else {
          <div class="grid grid-cols-2 gap-6">
            <div>
              <div class="text-[10px] uppercase tracking-wider text-muted mb-2">Invocations per model</div>
              @for (m of mm.by_model; track m.model) {
                <div class="grid grid-cols-[110px_1fr_72px] gap-3 items-center font-mono text-xs py-1">
                  <span class="text-muted truncate">{{ m.model }}</span>
                  <div class="h-3.5 bg-border/60 rounded-sm overflow-hidden">
                    <div class="h-full bg-accent" [style.width.%]="(m.invocations / invocationsMax()) * 100"></div>
                  </div>
                  <span class="text-right tabular-nums">{{ m.invocations | number }}</span>
                </div>
              }
            </div>
            <div>
              <div class="text-[10px] uppercase tracking-wider text-muted mb-2">Sessions per model</div>
              @for (m of mm.by_model; track m.model) {
                <div class="grid grid-cols-[110px_1fr_72px] gap-3 items-center font-mono text-xs py-1">
                  <span class="text-muted truncate">{{ m.model }}</span>
                  <div class="h-3.5 bg-border/60 rounded-sm overflow-hidden">
                    <div class="h-full bg-info" [style.width.%]="(m.sessions / sessionsMax()) * 100"></div>
                  </div>
                  <span class="text-right tabular-nums">{{ m.sessions | number }}</span>
                </div>
              }
            </div>
          </div>
          <div class="mt-5">
            <div class="text-[10px] uppercase tracking-wider text-muted mb-2">Tools per model</div>
            <table class="w-full text-xs font-mono">
              <thead>
                <tr class="text-left text-[10px] uppercase text-muted">
                  <th class="pb-1.5 font-medium">Model</th>
                  <th class="pb-1.5 font-medium">Tool</th>
                  <th class="pb-1.5 font-medium text-right">Count</th>
                </tr>
              </thead>
              <tbody>
                @for (c of mm.by_model_tool; track $index) {
                  <tr class="border-t border-border/40">
                    <td class="py-1 text-muted">{{ c.model }}</td>
                    <td class="py-1">{{ c.tool }}</td>
                    <td class="py-1 text-right tabular-nums">{{ c.count | number }}</td>
                  </tr>
                }
              </tbody>
            </table>
          </div>
        }
      } @else {
        <div class="text-muted text-xs font-mono py-4">Loading…</div>
      }
    </div>
  </section>

  <!-- SLASH COMMANDS -->
  <section class="panel">
    <div class="panel-title">Slash commands</div>
    <div class="panel-body">
      @if (slash().length === 0) {
        <div class="text-muted text-xs font-mono py-4">No slash commands detected yet.</div>
      } @else {
        @for (c of slash(); track c.name) {
          <div class="flex justify-between items-center font-mono text-xs py-1 border-b border-border/30">
            <span class="text-accent">/{{ c.name }}</span>
            <span class="tabular-nums text-muted">{{ c.count | number }}</span>
          </div>
        }
      }
    </div>
  </section>

  <!-- SUB-AGENTS -->
  <section class="panel">
    <div class="panel-title">Sub-agent usage</div>
    <div class="panel-body">
      @if (subs().length === 0) {
        <div class="text-muted text-xs font-mono py-4">No sub-agent (Task) invocations detected yet.</div>
      } @else {
        @for (a of subs(); track a.subagent_type) {
          <div class="flex justify-between items-center font-mono text-xs py-1 border-b border-border/30">
            <span>{{ a.subagent_type }}</span>
            <span class="tabular-nums text-muted">{{ a.invocations | number }}</span>
          </div>
        }
      }
    </div>
  </section>

</div>
```

Notes for the implementer:
- `brain` and `flask-conical` (the `Brain` and `FlaskConical` lucide icons) are both already registered in `web/src/app/core/icons.ts` — no icon-registry change needed.
- `bg-accent`, `bg-info`, `text-warn`, `text-muted`, `bg-border/60`, `border-border/40`, `bg-warn/5`, `border-warn/40` are all generated from the `@theme` color tokens in `web/src/styles.css` and are already used by other pages — no new CSS.
- `.crumb`, `.panel`, `.panel-title`, `.panel-body`, `.tabular-nums` are shared classes in `web/src/styles.css`.

- [ ] **Step 3: Build the web app**

```powershell
cd web; npm run build
```

Expected: `Application bundle generation complete.` The only warning should be the pre-existing `NG8113: DatePipe is not used within the template of FilesComponent` (unrelated). No errors.

- [ ] **Step 4: Run the web tests**

```powershell
cd web; npm test
```

Expected: `Test Files 3 passed (3)`, `Tests 14 passed (14)`. The `app.component.spec.ts` icon-registry smoke test still passes (no new shell icons).

- [ ] **Step 5: Commit**

```powershell
git add web/src/app/features/behaviour/behaviour.component.ts web/src/app/features/behaviour/behaviour.component.html
git commit -m "feat(web): restyle Behaviour page to design system + Experimental banner"
```

---

## Task 4: Full verification + push

- [ ] **Step 1: Full Rust suite**

```powershell
cd src-tauri; cargo test --features test-support
```

Expected: every suite green, including the new `jsonl::record` (7), `jsonl::reducer` (8), and `jsonl_privacy` (3) tests. No regressions in any other suite.

- [ ] **Step 2: Web build + tests (already run in Task 3 — re-confirm)**

```powershell
cd web; npm run build; npm test
```

Expected: build succeeds; 14/14 tests pass.

- [ ] **Step 3: Push**

```powershell
git push
```

This pushes the four commits onto `feature/jsonl-ingest` (PR #9).

- [ ] **Step 4: Note for the user**

The slash-command fix is retroactive only after a re-ingest: tell the user to click **Settings → "Ingest JSONL history"** once after this lands, so the ~39 real slash commands (`/config`, `/plugin`, `/reload-plugins`, …) are detected and the Behaviour page's Slash commands section populates.

---

## Self-review checklist (run before opening/updating the PR)

1. **Spec coverage:**
   - Restyle to design system (spec Part 1) → Task 3.
   - String-or-array `content` deserialiser (spec Part 2) → Task 1.
   - Reducer + privacy regression tests (spec Tests) → Task 2.
   - Experimental banner (spec Part 3) → Task 3, Step 2.
2. **No placeholders:** every step has concrete code or a concrete command.
3. **Type consistency:** `deserialize_content` returns `Vec<ContentBlock>`, matching the `Message.content` field type. `ContentBlock::Text { text: Some(_) }` matches the enum variant in `record.rs`. `invocationsMax` / `sessionsMax` are referenced in the template exactly as named in the `.ts`.
4. **Privacy:** the deserialiser change widens what the reducer *reads* (string content) but not what it *emits* — `DerivedEvent` still carries no free text, and `user_string_content_text_never_leaks` proves it empirically.
