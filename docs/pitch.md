# Andon

Claude Code is wired up for OpenTelemetry out of the box — every session emits cost, tokens by model, accept/reject decisions per tool call, files edited, the lot. Almost no one looks at it because the path to actually seeing the data is "deploy an OTel collector, pipe it to a warehouse, build dashboards," which nobody is going to do for their own CLI tool.

Andon is what happens when you skip all of that. It's a single `.exe`. You double-click it, it patches your Claude Code settings to point telemetry at localhost, and a tray icon shows up.

![Overview](images/overview.png)

Under the hood it's pretty boring on purpose:

- Receives OTLP directly from Claude Code on localhost — no collector to deploy.
- Also reads `~/.claude/projects/**/*.jsonl` so a fresh install populates months of past sessions in one pass — and so a few things you can't see in OTel at all (model mix per session, slash-command usage, sub-agent delegations) end up in the dashboard.
- Stores everything in an embedded SQLite file under your home dir.
- Serves the dashboard from the tray icon — cost trends, model mix, accept-rate by language, file heatmap, per-session drill-down, a **Behaviour** page (slash commands, sub-agents, model mix), an **Efficiency** page (prompt-cache savings, per-family cost rates, main vs subagent split), and a live OTLP debug feed.
- **Flags your spend.** Set a monthly budget and the tray icon repaints amber → red as your projected end-of-month cost crosses 80% / 100%; a one-shot desktop notification fires at each threshold.
- Works with any Claude Code plan — Pro, Max, Team, Enterprise, API key. Telemetry is client-side, so the plan doesn't matter.
- All listeners bind to `127.0.0.1`. No account, no auth.

> **Privacy, in plain terms.** <ins>**No secrets, code contents, or prompts are ever persisted.**</ins> Andon's JSONL parser reads transcript files locally to derive numeric and structural signals (token counts, tool names, file paths, slash command names), but the reducer drops all prompt and response text before any DB write. Andon also ingests the numeric/metadata signal Claude Code emits via OTLP — token counts, cost, accept/reject decisions, file paths, timings. <ins>**Nothing leaves the engineer's machine.**</ins> The SQLite file sits in your home directory and the OTLP listeners are bound to localhost.
>
> *For now.* The local-only model is deliberate for v1. A natural next step, **if and only if a team wants it**, is an opt-in mode where the listener and storage live on a shared host so a team or org can see a roll-up. That would be opt-in, configurable, and still wouldn't change what data is collected — only where it's stored.

### How this differs from ccusage and JSONL-based tools

`ccusage` (and similar tools that scrape `~/.claude/projects/<slug>/*.jsonl`) read the conversation transcripts Claude Code writes to disk and derive costs from token counts × a bundled pricing table. They're brilliant for "what did I spend yesterday" answers with zero setup, and they work retroactively on data already on disk.

Andon ingests **both** streams. The OpenTelemetry feed Claude Code emits when telemetry is enabled carries the cost number Claude Code itself computed, plus tool-decision events, file deltas, and session lifecycle signals — and no conversation text at all. The JSONL transcripts add retroactive coverage (sessions that ran before Andon was installed) and the behavioural signal OTel doesn't carry (per-session model mix, slash-command usage, sub-agent delegations). Where both cover the same session, OTLP wins — Andon dedups on Anthropic's per-API-call `requestId` so the two sources never double-count.

So they're complementary, not competing. Use `ccusage` for the one-shot "show me the number" question on machines that have never been instrumented. Use Andon for the persistent dashboard, the behavioural views (accept-rate by language, file heatmap, repo attribution), the live OTLP diagnostic feed, and the optional forwarder to your own collector.

A couple of other caveats worth knowing up front:

- It's single-machine today. Your dashboard shows your data — there's no team roll-up yet (see above).
- For OTLP-covered sessions, cost comes from Claude Code's own `claude_code.cost.usage` metric — its computation from token counts × its built-in pricing table. For JSONL-only sessions (typically historical backfill), Andon prices the token counts itself against a bundled rate table — same approach `ccusage` uses. Either way the figures are directionally right, not a replacement for Anthropic invoices.

Mostly I built it because I wanted to know what I was spending and whether the suggestions Claude was making were any good. Both questions get more interesting once you can see the numbers.

Repo: [`github.com/satish-krishna/andon`](https://github.com/satish-krishna/andon).
