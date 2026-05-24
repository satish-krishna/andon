//! Coach module — anti-pattern rules, scorecard, and Skill Finder.
//!
//! See `docs/superpowers/specs/2026-05-24-ai-engineering-coach-integration-design.md`
//! for the architecture and rule catalogue. The module reads existing
//! Andon tables plus `prompt_turns` and writes findings to
//! `coach_findings`.

pub mod queries;
pub mod rules;
pub mod engine;
pub mod score;
pub mod skill;
pub mod eval;

/// The five practice areas. Keep ordered — the UI renders left-to-right.
pub const PRACTICES: &[&str] = &["prompt", "hygiene", "review", "tool", "context"];

#[derive(Debug, thiserror::Error)]
pub enum CoachError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, CoachError>;
