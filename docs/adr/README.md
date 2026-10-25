# Architecture Decision Records

One file per irreversible-ish decision. Format: Context / Decision / Consequences / Alternatives.

| ADR | Decision | Status |
|---|---|---|
| 0001 | The core is a static binary that orchestrates engines as subprocesses | Accepted (v0.2) |
| 0002 | The Capability Fingerprint is the only source of hardware truth, and measured beats declared | Accepted (v0.2) |
| 0003 | Prediction uses a two-parameter roofline with an unbounded calibration coefficient | Accepted (v0.2) |
| 0004 | The catalog mediates upstream churn and records per-artifact arch coverage | Accepted (v0.2) |
| 0005 | Provisioning is transactional, isolated, content-pinned, and never escalates privilege implicitly | Accepted (v0.2) |
| 0006 | Receipts are the primary product artifact; state is plain files, not a database | Accepted (v0.2) |
| 0007 | Platform certification is tiered and gated on hardware we have actually run | Accepted (v0.2) |
| 0008 | Bundle-based runtimes get a separate adapter class from text-generation engines | Accepted (v0.2) |
| 0009 | The TUI is a first-class view over the core, built with Ratatui | Accepted (v0.3) |
