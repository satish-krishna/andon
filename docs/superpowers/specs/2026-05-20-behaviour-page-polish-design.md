# Behaviour page polish — Design

> Status: draft 2026-05-20 · author: SatishKrishna Pilla
> Builds on: the JSONL behavioural ingest work (PR #9, `feature/jsonl-ingest`).

## Motivation

The Behaviour page shipped functional but rough. Three problems:

1. **It ignores the app's design system.** Every other page (`files`, `diagnostics`, `overview`) uses the shared `.crumb` header, `.panel` / `.panel-title` / `.panel-body` cards, and semantic colour tokens (`text-muted`, `text-accent`, …) defined in `web/src/styles.css`. The Behaviour template uses a bare `<h1>`, raw `<section>`/`<h2>`, and hardcoded `text-zinc-400` / `text-zinc-500` Tailwind classes. It looks foreign.

2. **Slash commands are under-detected.** The transcript corpus contains 39 real `<command-name>` records, but only synthetic test rows reach the database. Real slash-command user records store `message.content` as a plain JSON **string** (`"<command-name>/config</command-name>\n<command-message>config</command-message>\n<command-args></command-args>"`), but `jsonl::record::Message.content` only deserializes a `Vec<ContentBlock>`. Serde cannot coerce a string into a `Vec`, so `#[serde(default)]` yields an empty vec and the `<command-name>` tag is never scanned.

3. **The page is not flagged as experimental.** Behavioural views are JSONL-derived, sparse, and subject to change. There is no visual signal of that.

## Non-goals

- No API, DTO, or route changes. The three `/api/behaviour/*` endpoints stay as-is.
- No change to the reducer's detection *logic* — `detect_slash_command`, sub-agent detection, model-mix queries are all correct. Only the record deserialiser changes.
- No restyle of other pages.
- Sub-agent usage is **not** broken — the corpus genuinely contains only 2 `Task` calls. No fix needed there.
- No new "experimental" framework — the banner is a single static element on one page.

## Part 1 — Behaviour page restyle

### Current state

`web/src/app/features/behaviour/behaviour.component.html` — a `<div class="p-6 space-y-8">` with a bare `<h1>`, three `<section>`s each headed by `<h2>`, and `<ul>`/`<table>` lists using `text-zinc-*` colours.

### Target

Rebuild the template against the design system (`web/src/styles.css`), matching `files.component.html` and `diagnostics.component.html`:

- **Page header** — replace `<h1>` with the standard `.crumb`:
  ```html
  <div class="crumb">
    <span class="flex items-center gap-1.5">
      <lucide-icon name="brain" class="w-3.5 h-3.5"></lucide-icon>Behaviour
    </span>
  </div>
  ```
  (`brain` is already registered in `web/src/app/core/icons.ts`.) The crumb has
  only the left span — no right-side count or status element; the page has three
  distinct sections and no single count is meaningful.

- **Content wrapper** — `<div class="px-6 py-5 flex flex-col gap-4">` (the diagnostics pattern).

- **Each of the three sections** — a `.panel`:
  ```html
  <section class="panel">
    <div class="panel-title">Model mix</div>
    <div class="panel-body"> … </div>
  </section>
  ```

- **Model mix** — inside the panel, two labelled bar lists (`Invocations per model`, `Sessions per model`) using the Overview "Cost by model" row pattern:
  `grid grid-cols-[110px_1fr_72px] gap-3 items-center font-mono text-xs` per row, with a proportional fill bar (`<div class="h-3.5 bg-border/60 rounded-sm overflow-hidden">` containing a width-% inner `<div>`). Bar width = `value / max(values) * 100`. "Tools per model" stays a table but adopts the shared table styling — `text-xs font-mono`, `border-b border-border/40` rows, muted header row, `tabular-nums` right-aligned counts. `track $index` is retained for the model-tool rows (composite key; see the JSONL gap-fill PR review).

- **Slash commands / Sub-agent usage** — labelled rows: mono name on the left, right-aligned `tabular-nums` mono count. Empty states use the muted "No data" treatment from other panels (`text-muted text-xs font-mono py-4`), not `text-zinc-500`.

- **Colours** — only semantic tokens: `text-muted`, `text-text`, `text-accent`, `text-ok` etc. No `zinc-*`.

### Component TypeScript

`behaviour.component.ts` is unchanged — it keeps its three signals and one-shot fetches, and `ChangeDetectionStrategy.OnPush` is already set. The restyle is template-only.

## Part 2 — Slash-command detection fix

### Change

In `src-tauri/src/jsonl/record.rs`, `Message.content` gains a custom deserializer accepting three JSON shapes:

| Input JSON | Result |
|---|---|
| array | `Vec<ContentBlock>` (current behaviour) |
| string | a one-element vec: `[ContentBlock::Text { text: Some(<string>) }]` |
| absent / `null` | empty vec |

Implementation: a `deserialize_with` function on the `content` field using `serde_json::Value` as the intermediate, or a small `Deserialize` visitor. The function lives in `record.rs` next to the struct.

`detect_slash_command` and all other `ContentBlock`-walking code is unchanged — a string prompt simply arrives as one `Text` block.

### Consequence

After this lands, re-running Settings → "Ingest JSONL history" detects the real slash commands. No migration; `slash_commands` rows are additive and the `WHERE NOT EXISTS` idempotency guard prevents duplicates.

A string-content user record was previously a hard parse error — the whole
JSONL line was dropped. It now parses, so the reducer also runs `reduce_user`
on it: if it is a session's first turn, a `SessionLifecycle` event is emitted
that it previously was not. For a JSONL-only session whose first transcript
line is a string-content record, this can move `started_at` earlier (to the
true session start) and supply `cwd` / `git_branch` / `service_version` from
that record. This is a correct retroactive fix; `INSERT OR IGNORE` means it
applies only to freshly-ingested or never-seen sessions, not to sessions
already in the database.

### Privacy

The reducer now also reads string-form user content. This does not weaken the trust boundary: `reduce_user` still only extracts the `<command-name>` / `<command-args>` tag values and drops everything else when it returns. `DerivedEvent` carries no free text. The change is covered by extending the property test (Part 2 tests below).

## Tests

- **`record.rs` unit test** — `parses_string_content_as_single_text_block`: a `user` record whose `message.content` is the string `"<command-name>/review</command-name>"` deserializes to one `ContentBlock::Text` carrying that string. Plus a guard that array-form content still works (existing tests already cover this) and that absent content yields an empty vec.

- **`reducer.rs` unit test** — `string_content_user_record_emits_slash_command`: a `user` record with string-form `content` containing a `<command-name>` tag produces a `DerivedEvent::SlashCommand`. This is the end-to-end proof of the bug fix.

- **`jsonl_privacy.rs` proptest** — add `user_string_content_text_never_leaks`: same shape as the existing `user_text_never_leaks` but the generated prompt is placed as string-form `content` rather than inside an array text block. Asserts the reducer output contains no substring of the prompt.

- **Web** — no new web tests; the existing `app.component.spec.ts` icon-registry smoke test still passes (`brain` is already registered). `npm run build` + `npm test` (14/14) must stay green.

## Part 3 — Experimental banner

A static banner rendered once, directly below the `.crumb` on the Behaviour page, mirroring the Diagnostics health-banner pattern:

```html
<div class="mx-6 mt-4 border border-warn/40 bg-warn/5 rounded-md px-4 py-2.5 flex items-center gap-2.5">
  <lucide-icon name="flask-conical" class="w-4 h-4 text-warn shrink-0"></lucide-icon>
  <div class="text-xs">
    <span class="text-warn font-medium">Experimental.</span>
    <span class="text-muted">Behavioural views are derived from JSONL transcripts and may be
    incomplete or change between releases.</span>
  </div>
</div>
```

`flask-conical` is already in `APP_ICONS`. No dismiss control — it is a permanent status marker, not a notification.

## Files touched

| File | Change |
|---|---|
| `web/src/app/features/behaviour/behaviour.component.html` | Full restyle: `.crumb`, `.panel` cards, semantic tokens, bar rows, styled table, styled empty states, Experimental banner. |
| `web/src/app/features/behaviour/behaviour.component.ts` | Unchanged (template-only restyle). |
| `src-tauri/src/jsonl/record.rs` | Custom `content` deserializer (string ∣ array ∣ absent) + unit test. |
| `src-tauri/src/jsonl/reducer.rs` | No logic change; add the string-content slash-command unit test. |
| `src-tauri/tests/jsonl_privacy.rs` | Add the string-content privacy proptest variant. |

**Not touched:** API routes, DTOs, other pages, the reducer's detection logic, the database schema.

## Risks

- **Restyle drift.** The restyle is "match the existing system," so the risk is low — the reference pages (`files`, `diagnostics`, `overview`) are the spec. Verification is visual plus the existing build/test gates.
- **Deserializer edge cases.** Claude Code could emit `content` in a shape neither string nor array (e.g. a bare object). The deserializer treats anything not-array / not-string as empty vec — consistent with the existing lenient-parse philosophy (unknown shapes never abort a line). A malformed record becomes a no-op, not an error.
- **Re-ingest required.** The slash-command fix is retroactive only after the user re-runs backfill. This is the existing model for JSONL ingest and needs no special handling.
