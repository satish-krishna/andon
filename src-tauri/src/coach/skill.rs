//! Prompt-signature normaliser + discovery pass + examples reader.

use blake3::Hasher;
use once_cell::sync::Lazy;
use regex::Regex;

// Static 32-byte key — built into the binary, stable across runs but
// not portable across installs.
const NORM_KEY: &[u8; 32] = b"andon-coach-skill-finder-key-v1!";

static PATH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
    r"(?:@[\w./\\-]+|(?:^|\s)(?:/|[A-Za-z]:\\)[\w./\\-]+)"
).unwrap());
static UUID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
).unwrap());
static SHA_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b[0-9a-fA-F]{7,40}\b").unwrap());
static NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d{4,}").unwrap());
static CODE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

fn normalise(text: &str) -> String {
    let s = text.to_lowercase();
    let s = CODE_RE.replace_all(&s, "<code>");
    let s = PATH_RE.replace_all(&s, "<path>");
    let s = UUID_RE.replace_all(&s, "<id>");
    let s = SHA_RE.replace_all(&s, "<id>");
    let s = NUM_RE.replace_all(&s, "<num>");
    let s = WS_RE.replace_all(s.trim(), " ").into_owned();
    s.chars().take(1024).collect()
}

pub fn norm_hash(text: &str) -> String {
    let n = normalise(text);
    let mut h = Hasher::new_keyed(NORM_KEY);
    h.update(n.as_bytes());
    h.finalize().to_hex().to_string()
}

// ---------------------------------------------------------------------------
// G2: Skill discovery pass
// ---------------------------------------------------------------------------

use std::sync::Arc;
use crate::coach::rules::DbPool;
use crate::coach::Result;
use crate::settings::CoachSettings;

pub fn discover_all(pool: &Arc<DbPool>, settings: &CoachSettings) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let day = 86_400_000i64;
    for lookback_days in [30, 90, 180] {
        discover_window(pool, settings, now - lookback_days * day, now)?;
    }
    Ok(())
}

fn discover_window(pool: &Arc<DbPool>, settings: &CoachSettings, from_ms: i64, to_ms: i64) -> Result<()> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT norm_hash,
                COUNT(*) AS occurrences,
                COUNT(DISTINCT session_id) AS session_count,
                MIN(ts) AS first_seen, MAX(ts) AS last_seen,
                (SELECT text FROM prompt_turns p2
                  WHERE p2.norm_hash = p.norm_hash
                  ORDER BY length ASC, ts ASC LIMIT 1) AS shortest_text,
                (SELECT command FROM prompt_turns p3
                  WHERE p3.norm_hash = p.norm_hash AND command IS NOT NULL
                  GROUP BY command
                  HAVING COUNT(DISTINCT command) = 1 LIMIT 1) AS unique_command
         FROM prompt_turns p
         JOIN sessions s USING (session_id)
         WHERE s.started_at >= ?1 AND s.started_at < ?2
         GROUP BY norm_hash
         HAVING occurrences >= ?3 AND session_count >= ?4",
    )?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let rows = stmt.query_map(
        rusqlite::params![
            from_ms, to_ms,
            settings.skill_min_occurrences as i64,
            settings.skill_min_sessions as i64,
        ],
        |r| Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
        )),
    )?;
    let collected: Vec<_> = rows.filter_map(|r| r.ok()).collect();
    drop(stmt);
    drop(conn);

    let mut conn2 = pool.get()?;
    let tx = conn2.transaction()?;
    for row in collected {
        let (hash, occ, sess, first, last, shortest, cmd) = row;
        let label = if let Some(c) = cmd.as_deref() {
            format!("/{}", c)
        } else {
            shortest.clone().unwrap_or_default()
                .chars().take(80).collect::<String>()
        };
        tx.execute(
            "INSERT INTO skill_opportunities
               (norm_hash, label, command, occurrences, session_count,
                first_seen, last_seen, window_start, window_end, computed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(norm_hash, window_start, window_end) DO UPDATE SET
               label = excluded.label,
               occurrences = excluded.occurrences,
               session_count = excluded.session_count,
               first_seen = excluded.first_seen,
               last_seen = excluded.last_seen,
               computed_at = excluded.computed_at",
            rusqlite::params![hash, label, cmd, occ, sess, first, last, from_ms, to_ms, now_ms],
        )?;
    }
    tx.commit()?;
    Ok(())
}
