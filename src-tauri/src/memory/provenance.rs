use rusqlite::{Connection, Result, params};
use serde::Serialize;

/// Sentinel session id for touches made by a human in Andon's UI, which have
/// no session. Kept out of the sessions FK graph deliberately (MIGRATION_V7).
pub const ANDON_USER: &str = "andon-user";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Update,
    Edit,
    Delete,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Create => "create",
            Action::Update => "update",
            Action::Edit => "edit",
            Action::Delete => "delete",
        }
    }

    /// Maps a hook's `tool_name` onto an action. `Write` reads as `create`
    /// because it replaces the whole file; a PostToolUse hook fires after the
    /// write and cannot tell whether the file already existed, so `create`
    /// means "written wholesale by the model", not "first ever touch".
    pub fn for_tool(tool: &str) -> Option<Action> {
        match tool {
            "Write" => Some(Action::Create),
            "Edit" | "MultiEdit" => Some(Action::Update),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Touch {
    pub session_id: String,
    pub action: String,
    pub ts: i64,
}

/// Appends a row. The ledger is append-only by design: there is deliberately no
/// function to remove rows, because a delete followed by a model re-create is
/// the churn signal this feature exists to collect.
pub fn record(
    conn: &Connection,
    session_id: &str,
    slug: &str,
    file: &str,
    action: Action,
    ts: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_provenance (session_id, project_slug, memory_file, action, ts)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session_id, slug, file, action.as_str(), ts],
    )?;
    Ok(())
}

/// Full touch history for a memory, most recent first.
pub fn touches(conn: &Connection, slug: &str, file: &str) -> Result<Vec<Touch>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, action, ts
           FROM memory_provenance
          WHERE project_slug = ?1 AND memory_file = ?2
          ORDER BY ts DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![slug, file], |r| {
        Ok(Touch { session_id: r.get(0)?, action: r.get(1)?, ts: r.get(2)? })
    })?;
    rows.collect()
}

/// The headline origin: what the memory says now is what the last session wrote.
pub fn last_touch(conn: &Connection, slug: &str, file: &str) -> Option<Touch> {
    touches(conn, slug, file).ok()?.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn db() -> Connection {
        let mut c = Connection::open_in_memory().expect("open in-memory db");
        crate::db::migrations::apply(&mut c).expect("migrations apply");
        c
    }

    #[test]
    fn action_maps_from_the_hook_tool_name() {
        assert_eq!(Action::for_tool("Write").map(|a| a.as_str()), Some("create"));
        assert_eq!(Action::for_tool("Edit").map(|a| a.as_str()), Some("update"));
        assert_eq!(Action::for_tool("MultiEdit").map(|a| a.as_str()), Some("update"));
        assert!(Action::for_tool("Bash").is_none());
    }

    #[test]
    fn touches_returns_most_recent_first() {
        let c = db();
        record(&c, "sess-a", "P", "m.md", Action::Create, 100).expect("record create");
        record(&c, "sess-b", "P", "m.md", Action::Update, 200).expect("record update");

        let got = touches(&c, "P", "m.md").expect("touches");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].session_id, "sess-b");
        assert_eq!(got[0].action, "update");
        assert_eq!(got[1].session_id, "sess-a");
    }

    #[test]
    fn last_touch_is_the_headline_origin() {
        let c = db();
        record(&c, "sess-a", "P", "m.md", Action::Create, 100).expect("record create");
        record(&c, "sess-b", "P", "m.md", Action::Update, 200).expect("record update");
        assert_eq!(last_touch(&c, "P", "m.md").expect("some touch").session_id, "sess-b");
    }

    /// `idx_memory_prov_file(project_slug, memory_file, ts DESC)` already returns
    /// rows in `ts DESC` order as an artifact of the index scan, so a test that
    /// only varies `ts` cannot tell whether `ORDER BY ts DESC, id DESC` is doing
    /// any work. Two rows sharing the same `ts` break that coincidence: the
    /// index's internal tiebreak is rowid ASCENDING, while the query's tiebreak
    /// is `id DESC`. Only a real `ORDER BY id DESC` produces last-inserted-first
    /// here; the index's natural scan order alone would produce the opposite.
    #[test]
    fn touches_breaks_ts_ties_by_most_recent_insert_first() {
        let c = db();
        record(&c, "sess-first", "P", "m.md", Action::Create, 100).expect("record first");
        record(&c, "sess-second", "P", "m.md", Action::Update, 100).expect("record second");

        let got = touches(&c, "P", "m.md").expect("touches");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].session_id, "sess-second", "last-inserted row must come first on a ts tie");
        assert_eq!(got[1].session_id, "sess-first");
    }

    #[test]
    fn last_touch_breaks_ts_ties_by_most_recent_insert_first() {
        let c = db();
        record(&c, "sess-first", "P", "m.md", Action::Create, 100).expect("record first");
        record(&c, "sess-second", "P", "m.md", Action::Update, 100).expect("record second");

        assert_eq!(
            last_touch(&c, "P", "m.md").expect("some touch").session_id,
            "sess-second",
            "last_touch must name the most recently inserted session on a ts tie"
        );
    }

    #[test]
    fn last_touch_is_none_for_a_pre_ledger_memory() {
        let c = db();
        assert!(last_touch(&c, "P", "never-seen.md").is_none());
    }

    #[test]
    fn rows_are_scoped_per_project() {
        let c = db();
        record(&c, "sess-a", "P1", "m.md", Action::Create, 100).expect("record");
        assert!(touches(&c, "P2", "m.md").expect("touches").is_empty());
    }

    #[test]
    fn deleting_a_memory_appends_history_rather_than_erasing_it() {
        // The churn signal this feature exists to collect: a human delete
        // followed by the model writing the fact back.
        let c = db();
        record(&c, "sess-a", "P", "m.md", Action::Create, 100).expect("model create");
        record(&c, ANDON_USER, "P", "m.md", Action::Delete, 200).expect("human delete");
        record(&c, "sess-b", "P", "m.md", Action::Create, 300).expect("model re-create");

        let got = touches(&c, "P", "m.md").expect("touches");
        assert_eq!(got.len(), 3, "history must survive the delete");
        assert_eq!(got[0].session_id, "sess-b");
        assert_eq!(got[1].session_id, ANDON_USER);
        assert_eq!(got[1].action, "delete");
    }
}
