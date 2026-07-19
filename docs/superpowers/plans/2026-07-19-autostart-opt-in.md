# Autostart Opt-In Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop force-enabling Windows logon autostart on every boot; make it genuinely opt-in via the existing Settings toggle, while grandfathering users who already have it.

**Architecture:** Extract the autostart decision into a pure `decide()` function (no registry I/O, unit-testable on every platform). Rewire `ensure_current()` to self-heal an existing Run-key entry but never create one — a fresh install with no entry is left untouched. The Run key's presence is the opt-in state; the already-built Settings toggle is the only writer.

**Tech Stack:** Rust, `rusqlite`/Tauri backend, `serde` for the outcome tag, `winreg` for the Windows registry. Tests run via `cargo test --features test-support` from `src-tauri/`.

## Global Constraints

- US English everywhere (behavior, color, organize).
- No `unwrap()` / `expect()` outside `main.rs` setup — this change adds neither.
- Conventional Commits, no emojis: `type(scope): subject`.
- TDD: failing test first, then implementation.
- All work on branch `feat/autostart-opt-in` (already created; spec already committed there).
- Windows / PowerShell shell. Run backend tests from the `src-tauri` directory.
- The single source of truth for autostart state is the registry value `andon` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Do not add a second store.

---

### Task 1: Gate `ensure_current()` behind opt-in via a pure `decide()`

**Files:**
- Modify: `src-tauri/src/autostart.rs` — add top-level `decide()`, reshape `EnsureOutcome`, rewire the Windows `ensure_current()`.
- Test: `src-tauri/src/autostart.rs` (inline `#[cfg(test)] mod tests`) — add `decide()` tests, update the serialization test.

**Interfaces:**
- Produces: `pub fn decide(registered: Option<&str>, want: &str) -> EnsureOutcome` at the `autostart` module level (compiled and tested on all platforms).
- Produces: `EnsureOutcome` variants become `Updated`, `AlreadyCorrect`, `NotEnabled`, `Unsupported` (the `Enabled` variant is removed; `NotEnabled` is added, serializing to `not_enabled`).
- Consumes: existing `current_exe()`, `registered_command()`, `enable()` inside the Windows `imp` module (unchanged signatures).

- [ ] **Step 1: Write the failing `decide()` tests**

Add these three tests inside the existing `#[cfg(test)] mod tests { ... }` block in `src-tauri/src/autostart.rs` (do NOT gate them with `#[cfg(not(windows))]` — they are pure and must run on every platform):

```rust
    // Pure policy: an absent Run-key value means the user has NOT opted in,
    // so ensure_current must do nothing.
    #[test]
    fn decide_absent_key_is_not_enabled() {
        let want = r#""C:\Apps\andon.exe""#;
        assert!(matches!(decide(None, want), EnsureOutcome::NotEnabled));
    }

    // Value already points at the current exe: nothing to do.
    #[test]
    fn decide_matching_key_is_already_correct() {
        let want = r#""C:\Apps\andon.exe""#;
        assert!(matches!(decide(Some(want), want), EnsureOutcome::AlreadyCorrect));
    }

    // Value present but stale (app moved/reinstalled): self-heal the path.
    // This is the grandfather path for existing opted-in users.
    #[test]
    fn decide_stale_key_is_updated() {
        let want = r#""C:\Apps\andon.exe""#;
        let stale = r#""C:\Old\andon.exe""#;
        assert!(matches!(decide(Some(stale), want), EnsureOutcome::Updated));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri; cargo test --features test-support decide_`
Expected: FAIL — compile error, `cannot find function 'decide'` and `no variant 'NotEnabled'`. (A compile failure is the valid "red" here.)

- [ ] **Step 3: Add the `decide()` function and the `NotEnabled` variant**

In `src-tauri/src/autostart.rs`, add `decide()` at the module top level (outside both `imp` modules, next to `EnsureOutcome`), so it is platform-independent:

```rust
/// Pure autostart policy — no registry access, so it is unit-testable on
/// every platform. `registered` is the current Run-key value (already
/// quoted) or `None` when the key is absent; `want` is the quoted command
/// we would register for the current exe. Absent means the user never opted
/// in, so we leave it alone; a stale value is self-healed.
pub fn decide(registered: Option<&str>, want: &str) -> EnsureOutcome {
    match registered {
        None => EnsureOutcome::NotEnabled,
        Some(existing) if existing == want => EnsureOutcome::AlreadyCorrect,
        Some(_) => EnsureOutcome::Updated,
    }
}
```

Then add the `NotEnabled` variant to the enum (keep `Enabled` for now so the crate still compiles — it is removed in Step 5):

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EnsureOutcome {
    Enabled,
    Updated,
    AlreadyCorrect,
    NotEnabled,
    Unsupported,
}
```

- [ ] **Step 4: Run the `decide()` tests to verify they pass**

Run: `cd src-tauri; cargo test --features test-support decide_`
Expected: PASS — `decide_absent_key_is_not_enabled`, `decide_matching_key_is_already_correct`, `decide_stale_key_is_updated` all green.

- [ ] **Step 5: Rewire `ensure_current()`, remove `Enabled`, update the serialization test**

Replace the Windows `ensure_current()` body (currently the `match registered_command()` block that calls `enable()` in the `None` arm) with a thin wrapper over `decide()`:

```rust
    /// Keep autostart pointing at the current exe *only if the user already
    /// enabled it*. A fresh install (no Run-key value) is left untouched —
    /// autostart is opt-in via the Settings toggle. If the user opted in and
    /// later moved or reinstalled the app, self-heal the registered path.
    pub fn ensure_current() -> Result<EnsureOutcome> {
        let want = current_exe()?;
        let outcome = decide(registered_command().as_deref(), &want);
        if matches!(outcome, EnsureOutcome::Updated) {
            enable()?;
        }
        Ok(outcome)
    }
```

Now remove the `Enabled` variant from `EnsureOutcome` (it is no longer constructed anywhere):

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EnsureOutcome {
    Updated,
    AlreadyCorrect,
    NotEnabled,
    Unsupported,
}
```

Update the existing `ensure_outcome_serializes_to_snake_case_tag` test — drop the `Enabled` case, add the `NotEnabled` case:

```rust
        let cases: &[(EnsureOutcome, &str)] = &[
            (EnsureOutcome::Updated, r#"{"outcome":"updated"}"#),
            (EnsureOutcome::AlreadyCorrect, r#"{"outcome":"already_correct"}"#),
            (EnsureOutcome::NotEnabled, r#"{"outcome":"not_enabled"}"#),
            (EnsureOutcome::Unsupported, r#"{"outcome":"unsupported"}"#),
        ];
```

- [ ] **Step 6: Run the full autostart test set to verify everything passes**

Run: `cd src-tauri; cargo test --features test-support autostart`
Expected: PASS — `registry_constants_are_correct`, `ensure_outcome_serializes_to_snake_case_tag`, and the three `decide_*` tests. (`non_windows_stubs_are_safe` is `#[cfg(not(windows))]` and only runs off-Windows.)

- [ ] **Step 7: Confirm the crate builds clean (no dead-code warning from the removed variant)**

Run: `cd src-tauri; cargo build`
Expected: builds with no new warnings referencing `EnsureOutcome` or `Enabled`.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/autostart.rs
git commit -m "feat(autostart): make logon autostart opt-in instead of forced

ensure_current() now self-heals an existing Run-key entry but never
creates one. A fresh install writes nothing until the user enables
autostart via the Settings toggle; existing opted-in users are
grandfathered by the stale-path self-heal. Decision logic extracted to
a pure decide() so it is unit-tested on every platform without touching
the real registry.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Manual verification (Windows, before release)

Automated tests cover `decide()`, but the registry side effect is only exercised at runtime. Before cutting a release, confirm on a Windows machine:

1. **Fresh-install simulation:** ensure no `andon` value exists under `HKCU\...\Run` (`reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v andon /f`), launch the app, then check the key is still absent (`reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v andon` → not found). Expected: boot does NOT create it.
2. **Opt-in:** toggle autostart on in Settings → key appears. Restart the app → key persists, unchanged.
3. **Grandfather / self-heal:** with the key present but pointing at a stale path, launch from the current path → value is rewritten to the current exe.
4. **Opt-out:** toggle autostart off in Settings → key removed. Restart the app → key stays absent.

---

## Release notes (paste into `gh release create --notes` at the next release cut)

Frame as a standing condition of unsigned distribution, not a one-off. There is no CHANGELOG file — per `docs/releasing.md`, notes are authored inline at release time.

```markdown
### Autostart is now opt-in

Andon no longer registers itself to start at Windows logon automatically.
Enable it from **Settings → Start at login** if you want it. If you already
had autostart on, it stays on — no action needed.

**Windows Defender note:** Andon ships as an unsigned binary, and Defender's
machine-learning heuristics can occasionally flag unsigned apps that set a
logon-start entry (a `...!ml` detection). This is a false positive. Making
autostart opt-in reduces how often it triggers; code signing is on the
roadmap as the durable fix. If you hit a block, you can restore the file from
quarantine and/or report it to Microsoft as a false positive.
```
