use andon_lib::coach::skill::norm_hash;

#[test]
fn case_and_whitespace_collapse() {
    assert_eq!(norm_hash("Package the extension"),
               norm_hash("  package   the   extension  "));
}

#[test]
fn paths_collapse() {
    assert_eq!(norm_hash("Refactor @src/foo.rs"),
               norm_hash("Refactor @lib/bar.rs"));
    assert_eq!(norm_hash("Refactor C:\\Users\\x\\foo.rs"),
               norm_hash("Refactor /home/y/bar.rs"));
}

#[test]
fn uuids_and_long_numbers_collapse() {
    let a = "Investigate run 550e8400-e29b-41d4-a716-446655440000";
    let b = "Investigate run f47ac10b-58cc-4372-a567-0e02b2c3d479";
    assert_eq!(norm_hash(a), norm_hash(b));

    let a = "PR #12345";
    let b = "PR #98765";
    assert_eq!(norm_hash(a), norm_hash(b));
}

#[test]
fn code_fences_drop_out() {
    let a = "Explain this:\n```rust\nfn foo(){}\n```";
    let b = "Explain this:\n```python\nprint(1)\n```";
    assert_eq!(norm_hash(a), norm_hash(b));
}

#[test]
fn very_long_inputs_truncate() {
    let pad = "abcd".repeat(2000);
    let a = format!("Plan the release. {}", pad);
    let b = format!("Plan the release. {} extra", pad);
    assert_eq!(norm_hash(&a), norm_hash(&b),
        "anything beyond 1024 chars of normalised input should not affect the hash");
}

#[test]
fn different_inputs_differ() {
    assert_ne!(norm_hash("package the extension"),
               norm_hash("ship the release"));
}
