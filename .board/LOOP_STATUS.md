# Ralph-loop status: parked (known blocked)

On 2026-04-16 the `run-manager.sh` / `run-executor.sh` launchers were
exercised with `MAX_ITERS=3 SLEEP_SECS=30`. `claude -p` consistently
hangs (>90s with 0% CPU and no stdout) when fed a prompt of more than
a few dozen bytes from within this repo, regardless of:

- `--permission-mode bypassPermissions` vs `--dangerously-skip-permissions`
- `--model opus` vs `sonnet`
- prompt delivered as argv vs piped on stdin
- cwd = repo vs cwd = /tmp
- `--setting-sources user` (skip project settings)

A tiny control prompt like `claude -p "say hello in 3 words"` returns
in ~6s, so the auth/session path works. Even the first 5 lines of
`prompts/executor.md` trigger the hang, while just the heading does
not. The content appears to cause either a server-side stall or a
client-side plugin-initialization deadlock.

`--bare` would isolate the cause but requires `ANTHROPIC_API_KEY`;
this account is on OAuth only. Debugging further is not a language
improvement, so the loops are **parked**.

## What is still useful

- The **board itself** (`tickets/`, `ROADMAP.md`, `README.md`) — we
  keep using it as the unit of work, even when a human operator is
  the executor.
- **Ticket discipline** — every code change still carries a `RES-NNN:`
  subject and ends with ticket state moving to `DONE/`.
- **The launcher scripts** stay checked in, ready to resume once the
  `claude -p` hang is understood.

## When you come back to this

First reproduction step:

```bash
time (timeout 30 claude -p --permission-mode bypassPermissions --model opus \
    "$(head -5 .board/prompts/executor.md)" < /dev/null)
```

Should return within ~10s; if it still hits the 30s timeout, the
issue has not been fixed. Try `--debug api` and capture stderr to a
file — we couldn't, because debug output never flushed before
timeout in the original investigation.
