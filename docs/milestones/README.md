# Milestones

Each milestone is independently demonstrable and has a hard exit gate. A milestone that cannot be demoed in under five minutes is not finished.

```
M0 Advisor ──▶ M1 Apply ──▶ M2 Tune & Serve ──▶ M3 Fleet & Air-gap
   no installs    rollback       autotune + daemon      bundles + drift
```

| Milestone | Objective | Duration | Exit gate |
|---|---|---|---|
| [M0](m0-advisor.md) | Prove we can tell a user what their machine can do, and be right, without touching their system. | 4–5 weeks | 12 acceptance tests green |
| [M1](m1-apply.md) | Take a plan and make it real, reversibly, with proof. | 6–8 weeks | 9 acceptance tests green |
| [M2](m2-tune-and-serve.md) | Improve on the plan automatically, and keep the result running. | 5–6 weeks | 8 acceptance tests green |
| [M3](m3-fleet-and-air-gap.md) | Make one good configuration reproducible across many machines, including disconnected ones. | 5–6 weeks | 7 acceptance tests green |
