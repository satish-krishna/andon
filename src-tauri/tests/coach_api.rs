mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn get_json(router: axum::Router, path: &str) -> (StatusCode, Value) {
    let resp = router.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
    let st = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (st, v)
}

#[tokio::test]
async fn coach_endpoints_smoke() {
    let (pool, _db_dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let (router, _r_dir) = common::test_router(&pool);

    // I1: scorecard
    let (s1, v1) = get_json(router.clone(), "/api/coach/scorecard?from=0&to=9999999999999").await;
    assert_eq!(s1, StatusCode::OK, "scorecard body: {v1}");
    assert_eq!(v1["practices"].as_array().unwrap().len(), 5, "5 practice rows");

    // I2: findings
    let (s2, v2) = get_json(router.clone(), "/api/coach/findings").await;
    assert_eq!(s2, StatusCode::OK);
    assert!(v2["items"].is_array());

    // I3: rules
    let (s3, v3) = get_json(router.clone(), "/api/coach/rules").await;
    assert_eq!(s3, StatusCode::OK);
    let rules = v3.as_array().unwrap();
    assert_eq!(rules.len(), 12, "11 active + 1 reserved");
    let reserved = rules.iter().filter(|r| r["reserved"].as_bool() == Some(true)).count();
    assert_eq!(reserved, 1);

    // I5: skills
    let (s4, _v4) = get_json(router.clone(), "/api/coach/skills?lookback=90d").await;
    assert_eq!(s4, StatusCode::OK);

    // I6: skill examples
    let (s5, _v5) = get_json(router.clone(), "/api/coach/skills/nonexistent/examples").await;
    assert_eq!(s5, StatusCode::OK, "empty examples should still be 200");

    // I8: settings GET
    let (s6, v6) = get_json(router.clone(), "/api/settings/coach").await;
    assert_eq!(s6, StatusCode::OK);
    assert!(v6["planning_commands"].as_array().unwrap().len() >= 1);
}

#[tokio::test]
async fn coach_skills_lookback_validation_rejects_garbage() {
    let (pool, _db_dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let (router, _r_dir) = common::test_router(&pool);

    let (st, _) = get_json(router, "/api/coach/skills?lookback=42d").await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn coach_rules_toggle_reserved_returns_400() {
    let (pool, _db_dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let (router, _r_dir) = common::test_router(&pool);

    let resp = router.oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/coach/rules/high-cancellation")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"enabled":false}"#))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn coach_rules_toggle_unknown_returns_404() {
    let (pool, _db_dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let (router, _r_dir) = common::test_router(&pool);

    let resp = router.oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/coach/rules/does-not-exist")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"enabled":false}"#))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn coach_rules_toggle_real_rule_works() {
    let (pool, _db_dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let (router, _r_dir) = common::test_router(&pool);

    let resp = router.oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/coach/rules/lazy-prompting")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"enabled":false}"#))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify it took effect.
    let enabled: i64 = pool.get().unwrap().query_row(
        "SELECT enabled FROM coach_rules WHERE id = 'lazy-prompting'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(enabled, 0);
}
