use andon_lib::coach::rules::{RULES, RuleKind};

#[test]
fn catalogue_has_exactly_eleven_active_rules_plus_one_reserved() {
    let active: Vec<_> = RULES.iter().filter(|r| !r.reserved).collect();
    assert_eq!(active.len(), 11, "11 active rules — 10 binary + 1 continuous");

    let reserved: Vec<_> = RULES.iter().filter(|r| r.reserved).collect();
    assert_eq!(reserved.len(), 1, "high-cancellation slot reserved");
    assert_eq!(reserved[0].id, "high-cancellation");
}

#[test]
fn every_active_rule_has_description_and_suggestion() {
    for r in RULES.iter().filter(|r| !r.reserved) {
        assert!(!r.description.is_empty(), "rule {} missing description", r.id);
        assert!(!r.suggestion.is_empty(), "rule {} missing suggestion", r.id);
    }
}

#[test]
fn exactly_one_continuous_rule_in_phase_1() {
    let cont: Vec<_> = RULES.iter()
        .filter(|r| !r.reserved && matches!(r.kind, RuleKind::Continuous))
        .collect();
    assert_eq!(cont.len(), 1);
    assert_eq!(cont[0].id, "model-diversity");
}
