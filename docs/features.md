# Features

A walkthrough of every page in the andon dashboard. All screenshots are full-page captures of the actual app against a working SQLite database.

## Overview

The default landing page. Designed to be the one place you check each morning.

![Overview](images/overview.png)

- **Range selector** — Today / This week / This month / Last 30 days / Custom. Every other chart on the page respects it.
- **Model filter** — toggle Opus / Sonnet / Haiku to isolate spend per model family.
- **Top KPIs** — month-to-date cost (with last-month-same-day comparison and end-of-month projection at current pace), session count, and a split token breakdown (input / output / cache).
- **The Tape** — a calendar-as-row visualisation of daily cost across the month. Today is marked, the y-axis is a money scale, future days are blanked.
- **Cost by model** — period total broken down by model so you can see the Opus-vs-Sonnet mix.
- **Accept rate by language** — horizontal bars sorted descending, with raw edit counts.
- **Active time** — wall-clock minutes you spent vs. minutes Claude Code spent computing.
- **Recent sessions** — last 6 sessions, click-through to detail.

## Sessions

Every Claude Code session andon has seen, filterable and sortable.

![Sessions](images/sessions.png)

- Same range + model filters as Overview, plus free-text search across session ID and file paths.
- Sort by time, cost, duration, or decision count.
- Rows expand inline to show files touched and decisions made in that session, or you can click into a full detail view.
- Accept rate per row is computed as `accepts / (accepts + rejects + aborts)`.

## Session detail

Click any session ID to drill in. Captures everything that happened in that one CLI session.

![Session detail](images/session-detail.png)

- Cost, token, and decision totals for the session.
- Timeline of tool decisions (accept / reject / abort) with the file and language they touched.
- Files modified with lines added / removed.
- Resource attributes captured at the start of the session (host, OS, terminal, Claude Code version).

The same view powers the standalone HTML reports you can export from the **Open report** button.

## Files

A repo-wide view of what Claude Code has been changing.

![Files](images/files.png)

- KPIs: files touched, total edits, lines added / removed, net change.
- Sortable file table with edits, accept rate, churn, language, last-touched-relative time.
- Filterable by language (rust / typescript / toml / ...) and by free-text path match.
- **Heatmap** at the bottom: every file sized by edit count and coloured by accept rate (red = low accept, green = high). Lets you spot files Claude Code keeps suggesting changes to that you keep rejecting.

## Diagnostics

A live OTLP debugger built into the app. Indispensable when you're not sure whether telemetry is flowing.

![Diagnostics](images/diagnostics.png)

- **Health** — overall status with seconds-since-last-event.
- **Counters** — records received this session, uptime, last session ID.
- **Listener binds** — confirms `:4317`, `:4318`, `:8765` are bound and serving.
- **Transports** — gRPC vs HTTP request counts split by `metrics` / `logs`.
- **Event counters** — every event name andon has seen this session with a count and a "View" button that filters the feed below.
- **Event feed** — live tail of every OTLP record received, with timestamp, name, session ID, and transport. Filterable, pausable.
- **Download report** — exports a self-contained HTML report of the current diagnostic state for sharing.

## Settings

Configuration, integration status, and danger zone. Sections are anchor-linked from the left rail.

![Settings](images/settings.png)

- **Integration** — shows whether andon has successfully patched `~/.claude/settings.json` and exposes a "Re-apply" button. Includes a copy-paste manual config snippet for users who prefer to hand-edit.
- **Ingestion** — global pause toggle. When paused, OTLP listeners stay bound but drop incoming payloads (so Claude Code never sees an error).
- **OTel forwarder** — opt-in re-emitter. Configure a downstream HTTP/protobuf endpoint, timeout, and custom headers. Test the connection before saving.
- **Data** — DB file path, "Open folder" button, and live row counts per table.
- **App** — launch-at-logon toggle (writes to `HKCU\Run`, no admin needed).
- **About** — version, stack, repo link.
- **Danger zone** — unpatch `settings.json` (revert the env vars) or restore from the `.andon-backup` snapshot andon took at first patch.
