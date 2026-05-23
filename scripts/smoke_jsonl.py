#!/usr/bin/env python3
"""
End-to-end smoke for the JSONL ingest path.

1. Writes a synthetic JSONL file under a temp <claude_home>/projects/<slug>/.
2. Calls POST /api/jsonl/backfill (Andon must be running).
3. Asserts that the response shows records_processed > 0 and
   that GET /api/sessions includes the synthetic session id.

Run: python smoke_jsonl.py
No deps beyond stdlib.
"""
import json
import os
import pathlib
import sys
import time
import urllib.request

API = "http://127.0.0.1:8765"


def post(path, body=None):
    data = json.dumps(body or {}).encode("utf-8")
    req = urllib.request.Request(
        f"{API}{path}",
        data=data,
        method="POST",
        headers={"content-type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())


def get(path):
    with urllib.request.urlopen(f"{API}{path}", timeout=10) as r:
        return json.loads(r.read())


def main():
    home = pathlib.Path(os.environ.get("USERPROFILE") or os.environ["HOME"])
    proj = home / ".claude" / "projects" / "andon-smoke-jsonl"
    proj.mkdir(parents=True, exist_ok=True)
    sid = f"smoke-{int(time.time())}"
    transcript = proj / f"{sid}.jsonl"
    lines = [
        {
            "type": "user",
            "sessionId": sid,
            "timestamp": "2026-05-19T10:00:00.000Z",
            "cwd": str(home),
            "gitBranch": "main",
            "version": "2.1.0",
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "<command-name>/foo</command-name>"}],
            },
        },
        {
            "type": "assistant",
            "sessionId": sid,
            "timestamp": "2026-05-19T10:00:01.000Z",
            "message": {
                "role": "assistant",
                "model": "claude-opus-4-7",
                "usage": {"input_tokens": 10, "output_tokens": 20},
                "content": [
                    {
                        "type": "tool_use",
                        "id": "u1",
                        "name": "Task",
                        "input": {"subagent_type": "Explore"},
                    }
                ],
            },
        },
    ]
    transcript.write_text("\n".join(json.dumps(l) for l in lines), encoding="utf-8")

    stats = post("/api/jsonl/backfill")
    print("backfill:", stats)
    assert stats["records_processed"] > 0, "no records processed"

    sessions = get("/api/sessions")
    ids = [s["session_id"] for s in sessions]
    assert sid in ids, f"session {sid} not in /api/sessions"
    print("OK — session present:", sid)


if __name__ == "__main__":
    sys.exit(main() or 0)
