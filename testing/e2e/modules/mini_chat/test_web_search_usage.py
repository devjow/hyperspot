"""Web search usage verification tests.

Exercises web search with real LLM, then reads the SQLite DB to verify
that quota_usage rows, turn records, and message tokens are consistent.

Prints a diagnostic table after each scenario.
"""

import math
import os
import sqlite3
import uuid

import pytest
import httpx

from .conftest import API_PREFIX, DB_PATH, DEFAULT_MODEL, STANDARD_MODEL, expect_done, stream_message

pytestmark = [pytest.mark.openai, pytest.mark.online_only]


# ── DB helpers (same pattern as test_full_scenario.py) ──────────────────────

def _to_blob(value):
    if isinstance(value, str):
        try:
            return uuid.UUID(value).bytes
        except ValueError:
            pass
    return value


def query_db(sql: str, params: tuple = ()) -> list[dict]:
    if not os.path.exists(DB_PATH):
        pytest.skip(f"DB not found at {DB_PATH}")
    conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    blob_params = tuple(_to_blob(p) for p in params)
    try:
        rows = conn.execute(sql, blob_params).fetchall()
        return [dict(r) for r in rows]
    finally:
        conn.close()


def ceil_div(a: int, b: int) -> int:
    """Integer ceiling division matching Rust's ceil_div_checked."""
    if a == 0 or b == 0:
        return 0
    return (a + b - 1) // b


def expected_credits_micro(input_tokens: int, output_tokens: int, in_mult: int, out_mult: int) -> int:
    """Replicate credits_micro_checked from credit_arithmetic.rs."""
    divisor = 1_000_000
    return ceil_div(input_tokens * in_mult, divisor) + ceil_div(output_tokens * out_mult, divisor)


# Model multipliers from mini-chat.yaml (micro values)
MODEL_MULTIPLIERS = {
    "gpt-5.2": (3_000_000, 15_000_000),
    "gpt-5-mini": (1_000_000, 3_000_000),
    "gpt-5-nano": (500_000, 1_500_000),
}


def print_usage_table(label: str, turns: list[dict], messages: list[dict], quota: list[dict]):
    """Print a diagnostic table to stdout."""
    print(f"\n{'=' * 80}")
    print(f"  {label}")
    print(f"{'=' * 80}")

    if turns:
        print(f"\n  TURNS ({len(turns)} rows)")
        print(f"  {'state':<12} {'model':<14} {'reserve_tok':>11} {'max_out':>8} {'reserved_cr':>12}")
        for t in turns:
            print(
                f"  {t['state']:<12} "
                f"{(t.get('effective_model') or '?'):<14} "
                f"{t.get('reserve_tokens', '?'):>11} "
                f"{t.get('max_output_tokens_applied', '?'):>8} "
                f"{t.get('reserved_credits_micro', '?'):>12}"
            )

    if messages:
        print(f"\n  ASSISTANT MESSAGES ({len(messages)} rows)")
        print(f"  {'input_tok':>10} {'output_tok':>11} {'content_len':>12}")
        for m in messages:
            print(
                f"  {m['input_tokens']:>10} "
                f"{m['output_tokens']:>11} "
                f"{len(m['content']):>12}"
            )

    if quota:
        print(f"\n  QUOTA_USAGE ({len(quota)} rows)")
        print(
            f"  {'period':<8} {'bucket':<12} {'spent_cr':>12} {'reserved_cr':>12} "
            f"{'calls':>6} {'in_tok':>8} {'out_tok':>9} {'ws_calls':>9}"
        )
        for q in quota:
            print(
                f"  {q['period_type']:<8} "
                f"{q['bucket']:<12} "
                f"{q['spent_credits_micro']:>12} "
                f"{q['reserved_credits_micro']:>12} "
                f"{q['calls']:>6} "
                f"{q['input_tokens']:>8} "
                f"{q['output_tokens']:>9} "
                f"{q['web_search_calls']:>9}"
            )

    print(f"{'=' * 80}\n")


class TestWebSearchUsageAccounting:
    """Verify that web search turns produce correct DB records and credit math."""

    def test_web_search_usage_correct(self, server):
        """Single web-search turn: verify turns, messages, and quota_usage rows."""
        # Snapshot quota before this test (daily+total only — avoid double-counting monthly)
        quota_before = query_db(
            "SELECT * FROM quota_usage WHERE bucket = 'total' AND period_type = 'daily'"
        )
        spent_before = sum(q["spent_credits_micro"] for q in quota_before)
        calls_before = sum(q["calls"] for q in quota_before)
        ws_calls_before = sum(q["web_search_calls"] for q in quota_before)
        in_tokens_before = sum(q["input_tokens"] for q in quota_before)
        out_tokens_before = sum(q["output_tokens"] for q in quota_before)

        # Create chat (default model = gpt-5.2)
        resp = httpx.post(f"{API_PREFIX}/chats", json={})
        assert resp.status_code == 201
        chat = resp.json()
        chat_id = chat["id"]
        model = chat["model"]
        assert model == DEFAULT_MODEL

        # Send web search message
        rid = str(uuid.uuid4())
        status, events, _ = stream_message(
            chat_id,
            "Search the web: what is the current population of Tokyo?",
            web_search={"enabled": True},
            request_id=rid,
        )
        assert status == 200
        done = expect_done(events)

        # Extract SSE usage
        sse_usage = done.data["usage"]
        sse_input = sse_usage["input_tokens"]
        sse_output = sse_usage["output_tokens"]

        # Count tool events
        tool_events = [e for e in events if e.event == "tool"]
        ws_tool_starts = [
            e for e in tool_events
            if isinstance(e.data, dict)
            and e.data.get("phase") == "start"
            and e.data.get("name") in ("web_search", "web_search_preview")
        ]
        ws_tool_dones = [
            e for e in tool_events
            if isinstance(e.data, dict)
            and e.data.get("phase") == "done"
            and e.data.get("name") in ("web_search", "web_search_preview")
        ]

        # ── Read DB ──
        turns = query_db(
            "SELECT * FROM chat_turns WHERE chat_id = ? AND request_id = ?",
            (chat_id, rid),
        )
        asst_msgs = query_db(
            "SELECT * FROM messages WHERE chat_id = ? AND role = 'assistant' AND deleted_at IS NULL ORDER BY created_at",
            (chat_id,),
        )
        quota_after = query_db("SELECT * FROM quota_usage")

        # Print diagnostic table
        print_usage_table(
            f"Web Search Turn (model={model}, rid={rid[:8]}...)",
            turns, asst_msgs, quota_after,
        )

        # ── Turn assertions ──
        assert len(turns) == 1
        t = turns[0]
        assert t["state"] == "completed"
        assert t["effective_model"] == model
        assert t["reserve_tokens"] is not None and t["reserve_tokens"] > 0
        assert t["reserved_credits_micro"] is not None and t["reserved_credits_micro"] > 0

        # ── Message assertions ──
        assert len(asst_msgs) >= 1
        m = asst_msgs[-1]  # most recent assistant message
        assert m["input_tokens"] == sse_input, (
            f"DB input_tokens ({m['input_tokens']}) != SSE ({sse_input})"
        )
        assert m["output_tokens"] == sse_output, (
            f"DB output_tokens ({m['output_tokens']}) != SSE ({sse_output})"
        )

        # ── Quota assertions (daily+total only — settlement writes both daily & monthly) ──
        total_daily = [
            q for q in quota_after
            if q["bucket"] == "total" and q["period_type"] == "daily"
        ]
        assert len(total_daily) >= 1

        spent_after = sum(q["spent_credits_micro"] for q in total_daily)
        calls_after = sum(q["calls"] for q in total_daily)
        ws_calls_after = sum(q["web_search_calls"] for q in total_daily)
        in_tokens_after = sum(q["input_tokens"] for q in total_daily)
        out_tokens_after = sum(q["output_tokens"] for q in total_daily)

        # Credits formula: ceil(input * in_mult / 1M) + ceil(output * out_mult / 1M)
        in_mult, out_mult = MODEL_MULTIPLIERS[model]
        expected_credits = expected_credits_micro(sse_input, sse_output, in_mult, out_mult)

        spent_delta = spent_after - spent_before
        calls_delta = calls_after - calls_before
        ws_delta = ws_calls_after - ws_calls_before
        in_delta = in_tokens_after - in_tokens_before
        out_delta = out_tokens_after - out_tokens_before

        print("  CREDIT VERIFICATION:")
        print(f"    SSE tokens:      input={sse_input}, output={sse_output}")
        print(f"    Multipliers:     in={in_mult}, out={out_mult}")
        print(f"    Expected credits: {expected_credits}")
        print(f"    Actual delta:     {spent_delta}")
        print(f"    Calls delta:      {calls_delta}")
        print(f"    WS calls delta:   {ws_delta}")
        print(f"    Token deltas:     in={in_delta}, out={out_delta}")
        print()

        # Verify credit calculation matches
        assert spent_delta == expected_credits, (
            f"Credit mismatch: spent_delta={spent_delta} != expected={expected_credits} "
            f"(in={sse_input}*{in_mult} + out={sse_output}*{out_mult})"
        )

        # Verify call count incremented
        assert calls_delta >= 1

        # Verify web_search_calls incremented (should match completed tool calls)
        assert ws_delta >= 1, (
            f"web_search_calls not incremented: delta={ws_delta}, "
            f"tool done events={len(ws_tool_dones)}"
        )
        assert ws_delta == len(ws_tool_dones), (
            f"web_search_calls delta ({ws_delta}) != tool done events ({len(ws_tool_dones)})"
        )

        # Verify token deltas match SSE
        assert in_delta == sse_input, (
            f"input_tokens delta ({in_delta}) != SSE ({sse_input})"
        )
        assert out_delta == sse_output, (
            f"output_tokens delta ({out_delta}) != SSE ({sse_output})"
        )

        # Verify no stuck reserves
        for q in quota_after:
            assert q["reserved_credits_micro"] == 0, (
                f"Stuck reserve: bucket={q['bucket']} period={q['period_type']} "
                f"reserved={q['reserved_credits_micro']}"
            )

    def test_non_websearch_has_zero_ws_calls(self, server):
        """A normal turn (no web_search) should NOT increment web_search_calls."""
        quota_before = query_db(
            "SELECT * FROM quota_usage WHERE bucket = 'total' AND period_type = 'daily'"
        )
        ws_before = sum(q["web_search_calls"] for q in quota_before)

        resp = httpx.post(f"{API_PREFIX}/chats", json={})
        chat_id = resp.json()["id"]

        status, events, _ = stream_message(chat_id, "What is 2+2? Answer in one word.")
        assert status == 200
        expect_done(events)

        quota_after = query_db(
            "SELECT * FROM quota_usage WHERE bucket = 'total' AND period_type = 'daily'"
        )
        ws_after = sum(q["web_search_calls"] for q in quota_after)

        assert ws_after == ws_before, (
            f"web_search_calls changed without web_search: before={ws_before}, after={ws_after}"
        )

    def test_web_search_credits_match_model_multipliers(self, chat_with_model, server):
        """Verify credit formula for gpt-5-mini with web search.

        When actual tokens exceed reserve_tokens by more than the overshoot
        tolerance factor (default 1.1), credits are capped at reserved_credits_micro.
        """
        chat = chat_with_model(STANDARD_MODEL)
        chat_id = chat["id"]

        quota_before = query_db(
            "SELECT * FROM quota_usage WHERE bucket = 'total' AND period_type = 'daily'"
        )
        spent_before = sum(q["spent_credits_micro"] for q in quota_before)

        rid = str(uuid.uuid4())
        status, events, _ = stream_message(
            chat_id,
            "Search the web: who invented the telephone?",
            web_search={"enabled": True},
            request_id=rid,
        )
        assert status == 200
        done = expect_done(events)

        sse_input = done.data["usage"]["input_tokens"]
        sse_output = done.data["usage"]["output_tokens"]

        quota_after = query_db(
            "SELECT * FROM quota_usage WHERE bucket = 'total' AND period_type = 'daily'"
        )
        spent_after = sum(q["spent_credits_micro"] for q in quota_after)
        spent_delta = spent_after - spent_before

        in_mult, out_mult = MODEL_MULTIPLIERS[STANDARD_MODEL]
        formula_credits = expected_credits_micro(sse_input, sse_output, in_mult, out_mult)

        turns = query_db("SELECT * FROM chat_turns WHERE chat_id = ? AND request_id = ?", (chat_id, rid))
        asst_msgs = query_db(
            "SELECT * FROM messages WHERE chat_id = ? AND role = 'assistant' AND deleted_at IS NULL",
            (chat_id,),
        )
        quota_rows = query_db("SELECT * FROM quota_usage")
        print_usage_table(f"Web Search ({STANDARD_MODEL})", turns, asst_msgs, quota_rows)

        # Determine expected: formula or overshoot-capped at reserved_credits_micro
        assert len(turns) == 1
        t = turns[0]
        reserve_tokens = t["reserve_tokens"]
        reserved_credits = t["reserved_credits_micro"]
        actual_tokens = sse_input + sse_output
        overshoot_tolerance = 1.1  # QuotaConfig default

        if actual_tokens > reserve_tokens:
            overshoot_factor = actual_tokens / reserve_tokens
            if overshoot_factor > overshoot_tolerance:
                expected = reserved_credits
                capped = True
            else:
                expected = formula_credits
                capped = False
        else:
            expected = formula_credits
            capped = False

        print(f"  CREDIT VERIFICATION ({STANDARD_MODEL}):")
        print(f"    SSE tokens: input={sse_input}, output={sse_output}")
        print(f"    actual_tokens={actual_tokens}, reserve_tokens={reserve_tokens}")
        print(f"    Multipliers: in={in_mult}, out={out_mult}")
        print(f"    Formula credits: {formula_credits}")
        print(f"    Overshoot capped: {capped} (factor={actual_tokens / max(reserve_tokens, 1):.2f})")
        print(f"    Expected (after cap): {expected}, Actual delta: {spent_delta}")
        print()

        assert spent_delta == expected, (
            f"Credit mismatch for {STANDARD_MODEL}: "
            f"delta={spent_delta} != expected={expected} "
            f"(formula={formula_credits}, capped={capped})"
        )
