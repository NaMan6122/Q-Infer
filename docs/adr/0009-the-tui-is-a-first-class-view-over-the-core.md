# ADR-0009 — The TUI is a first-class view over the core, built with Ratatui

- **Status:** Accepted (v0.3)
- **Date:** 2026-10-26
- **Deciders:** engineering, product
- **Evidence:** PRD §7.8, §10.4; SDD §4.4; `spikes/m0/`
- **Review trigger:** If the TUI needs a network socket or a second process to function

## Context

The PRD's §10.4 sketches a TUI wizard and FR-51 requires one. That framing is too narrow: it treats the terminal UI as an installer wizard that disappears once setup is done.

The actual usage pattern is different. A bare-metal box is configured *occasionally* and observed *continuously* — is it still serving, is it throttling, did the quant fall back to CPU, is this machine slower than the identical one next to it. Those are dashboard questions, not wizard questions, and today they are answered by `nvidia-smi` in one terminal, `powermon` in another, and a `tail -f` in a third.

We also have a constraint that most dashboard products do not: ADR-0001 forbids the core from linking ML runtimes, and ADR-0005 forbids a daemon that silently holds privilege. A TUI that needs a background service to serve it data would violate the spirit of both.

## Decision

Ship a **TUI dashboard** that renders directly from the core's in-process state, using **Ratatui** over **Crossterm**.

Key properties:

1. **Single process.** `qinfer` with no subcommand opens the dashboard. It reads the same fingerprint, plans, receipts and calibration store that the CLI writes. No socket, no IPC, no second binary.
2. **Read-mostly.** The dashboard observes by default. Mutating actions (apply a plan, start tuning) are explicit confirmations, never a stray keystroke. `q` always quits; nothing destructive is bound to a bare key.
3. **Degrades to a pipe.** Every panel has a `--json` equivalent in the CLI. The TUI is a view, never the only way to obtain a fact. This keeps it usable over SSH, in CI, and by anyone who does not want a full-screen app.
4. **Live, but honest.** The Bench panel shows measured values with their spread and cache state. It must never render a single point estimate as if it were stable (PRD §5.5 noise discipline).
5. **Renders on a 80×24 terminal.** A dashboard that requires a 4K terminal is a demo, not a tool. Below 80 columns, panels collapse in a defined order rather than wrapping into mush.

## Consequences

**Positive**

- The "am I getting what I paid for?" question gets a permanent, glanceable answer.
- No new process model, no new protocol, no daemon to secure.
- Shared types with the CLI mean the dashboard cannot drift from the data model — it deserialises `cfp@1` / `plan@1` / `receipt@1` directly.
- Ratatui is pure Rust, MIT-licensed, and compiles into the same static binary, so the "single binary" promise in ADR-0001 survives.

**Negative / costs**

- A TUI is a real surface with real accessibility obligations; `--plain` and `--no-color` are P0, not extras.
- Full-screen rendering over slow SSH links can be unpleasant; we need a `--refresh` budget and a low-motion mode.
- Ratatui's API moves between minor versions; the TUI is pinned and upgraded deliberately, not automatically.
- Terminal size edge cases (resize, tmux panes) are a permanent source of small bugs.

## Alternatives considered

- **A local web UI (axum + React/Svelte).** Rejected for v1: needs a port, a browser, and a server process; hostile to SSH and to air-gapped policy. Revisit only if fleet views demand it (M3+).
- **A wizard-only TUI that exits after setup.** Rejected: answers the least valuable question (installation) and abandons the most valuable one (is it healthy).
- **No TUI; CLI + `watch`.** Rejected: composable but unreadable; five panes of `jq` output is not a dashboard.
- **Ratatui + a daemon over a Unix socket.** Rejected: reintroduces the process model ADR-0001 avoids.

## Notes

The dashboard's first job is not to be pretty. It is to make three facts unmissable: **what this machine is**, **what we predicted**, and **what it actually measured**. Every panel that does not serve one of those three is decoration and should be cut.

Implementation note: the M0 spike already emits `fingerprint.json` and `results.json`; the dashboard must render those unmodified so the design is testable before the planner exists.
