#!/usr/bin/env python3
"""Minimal Python client for the Resilient MCP HTTP wrapper.

Standard-library only (urllib.request) — no extra dependencies.
See docs/MCP_CLIENTS.md for the full write-up and curl equivalents.

Usage:
    rz mcp --http-port 8080 &
    python3 examples/mcp_client.py rz_run '{"source": "println(42)"}'
"""
import json
import sys
import urllib.error
import urllib.request

# Bypass system proxy auto-detection (e.g. macOS System Configuration),
# which can otherwise intercept plain http://127.0.0.1 requests and break
# urlopen's default opener even with no proxy actually configured.
_opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def call_tool(base_url: str, tool: str, tool_input: dict) -> dict:
    payload = json.dumps({"tool": tool, "input": tool_input}).encode("utf-8")
    request = urllib.request.Request(
        f"{base_url}/mcp/call",
        data=payload,
        headers={"content-type": "application/json"},
        method="POST",
    )
    try:
        with _opener.open(request, timeout=15) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        return json.loads(exc.read().decode("utf-8"))


def health(base_url: str) -> dict:
    with _opener.open(f"{base_url}/health", timeout=5) as response:
        return json.loads(response.read().decode("utf-8"))


def main() -> int:
    base_url = "http://127.0.0.1:8080"
    tool = sys.argv[1] if len(sys.argv) > 1 else "rz_run"
    tool_input = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {"source": "println(42)"}

    print("health:", health(base_url))
    result = call_tool(base_url, tool, tool_input)
    print(json.dumps(result, indent=2))
    return 0 if result.get("status") == "ok" else 1


if __name__ == "__main__":
    sys.exit(main())
