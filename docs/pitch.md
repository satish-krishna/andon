# Andon

Claude Code is wired up for OpenTelemetry out of the box — every session emits cost, tokens by model, accept/reject decisions per tool call, files edited, the lot. Almost no one looks at it because the path to actually seeing the data is "deploy an OTel collector, pipe it to a warehouse, build dashboards," which nobody is going to do for their own CLI tool.

Andon is what happens when you skip all of that. It's a single `.exe`. You double-click it, it patches your Claude Code settings to point telemetry at localhost, and a tray icon shows up.

![Overview](images/overview.png)

Under the hood it's pretty boring on purpose:

- Receives OTLP directly from Claude Code on localhost — no collector to deploy.
- Stores everything in an embedded SQLite file under your home dir.
- Serves the dashboard from the tray icon — cost trends, model mix, accept-rate by language, file heatmap, per-session drill-down, and a live OTLP debug feed for when you're not sure if telemetry is flowing.
- Works with any Claude Code plan — Pro, Max, Team, Enterprise, API key. Telemetry is client-side, so the plan doesn't matter.
- All listeners bind to `127.0.0.1`. No account, no auth.

> **Privacy, in plain terms.** <ins>**No secrets, code contents, or prompts are ever read or stored.**</ins> Andon only ingests the numeric/metadata signal Claude Code emits — token counts, cost, accept/reject decisions, file paths, timings. <ins>**Nothing leaves the engineer's machine.**</ins> The SQLite file sits in your home directory and the OTLP listeners are bound to localhost.
>
> *For now.* The local-only model is deliberate for v1. A natural next step, **if and only if a team wants it**, is an opt-in mode where the listener and storage live on a shared host so a team or org can see a roll-up. That would be opt-in, configurable, and still wouldn't change what data is collected — only where it's stored.

### How this differs from ccusage and JSONL-based tools

`ccusage` (and similar tools that scrape `~/.claude/projects/<slug>/*.jsonl`) read the conversation transcripts Claude Code writes to disk and derive costs from token counts × a bundled pricing table. They're brilliant for "what did I spend yesterday" answers with zero setup, and they work retroactively on data already on disk.

Andon ingests a different stream — the OpenTelemetry feed Claude Code emits when telemetry is enabled. That stream carries the cost number Claude Code itself computed, plus tool-decision events, file deltas, and session lifecycle signals — and no conversation text at all. Different data, different shape.

So they're complementary, not competing. Use `ccusage` for the one-shot "show me the number" question on machines that have never been instrumented. Use Andon for the persistent dashboard, the behavioural views (accept-rate by language, file heatmap, repo attribution), the live OTLP diagnostic feed, and the optional forwarder to your own collector.

A couple of other caveats worth knowing up front:

- It's single-machine today. Your dashboard shows your data — there's no team roll-up yet (see above).
- The cost numbers come from the `claude_code.cost.usage` metric Claude Code emits — i.e. Claude Code's own computation from token counts × its built-in pricing table, not from Anthropic invoices. Close enough to be directionally right, not a replacement for billing.

Mostly I built it because I wanted to know what I was spending and whether the suggestions Claude was making were any good. Both questions get more interesting once you can see the numbers.

Repo: [`github.com/satish-krishna/andon`](https://github.com/satish-krishna/andon).
