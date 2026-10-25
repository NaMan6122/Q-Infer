# ADR-0001 — The core is a static binary that orchestrates engines as subprocesses

- **Status:** Accepted (v0.2)
- **Date:** 2026-10-26
- **Deciders:** engineering, product
- **Evidence:** PRD §11.1, §16 Q1
- **Review trigger:** If subprocess orchestration costs >2 s of latency on any Tier-A path

## Context

Q-Infer must support inference runtimes that cannot coexist in one process: vLLM needs a specific torch/CUDA build, MLX needs arm64-only wheels, llama.cpp is a C++ binary, QNN is a vendor blob. Importing any of them into our address space would pin our dependency tree to theirs, make our binary platform-specific, and force a release every time an engine moves.

There is also a bootstrapping problem: we must be able to *plan for* a container on a machine that has no CUDA toolchain installed.

## Decision

The core ships as a single statically linked binary (Rust) with no ML dependencies. Engines are spawned as child processes. Where an engine is Python-based, we run it inside a managed environment that we created and pinned — never the user's ambient Python. The core communicates with engines over their native CLI and HTTP surfaces, and never links to them.

## Consequences

**Positive**

- The binary stays ~15–30 MB with no interpreter prerequisite.
- We can support an engine we cannot import on the host.
- Uniform supervision, uniform health checks, uniform metrics.
- Engine upgrades do not force a core release.

**Negative / costs**

- No in-process function calls: every interaction is a subprocess boundary or an HTTP call, so taxonomy of errors is ours to build.
- Startup and teardown need explicit lifecycle management (zombie processes, signal propagation).
- We cannot reuse engine internals for measurement; we must parse their output.

## Alternatives considered

- **Python CLI (fastest to build).** Rejected: caps the audience at people with a working Python environment, which is precisely the group that needs the least help, and inherits CUDA/torch conflicts.
- **Link engine libraries directly.** Rejected: N engines × M platforms becomes an ABI matrix we cannot maintain.
- **Go instead of Rust.** Viable; Rust chosen for predictable cross-compilation and static linking, with Go acceptable if velocity demands it.

## Notes

This is the hardest decision to reverse, which is why it is ADR-0001.
