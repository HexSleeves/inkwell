# Spike 003: Orca-Backed wave-ship Execution Substrate

## Problem

`wave-ship` runs the whole fleet inside **one** Workflow process. Every card is an
in-process `workflow()` / `agent()` call. The Workflow harness has a no-progress
watchdog (~180s) that reaps a subagent producing no tool output — and a long Build
(codex implementing a feature) routinely goes minutes without output. When the
watchdog reaps a builder, or the single orchestrator process stalls/compacts, the
**entire fleet freezes**. This is the documented `wave-ship agent-stall` in this
env; the fallback today is a manual codex-worktree loop.

Root cause: execution and coordination share one process. Orca's model decouples
them — workers are separate, runtime-tracked terminal processes; coordination is
external task-state. A stalled worker cannot freeze the coordinator.

## Decision

Add an execution backend to wave-ship: `backend: "workflow" | "orca"`.

- `"workflow"` (default, current): cards run as in-process `workflow(SHIP_REF, …)`.
- `"orca"`: each card's heavy work (Build + Review → green PR) runs as a **codex /
  claude worker in an isolated Orca worktree**, coordinated via `orca orchestration`
  task-state + `check --wait`. wave-ship's JS scheduler stays the coordinator brain —
  DAG readiness (Spike-B), migration serialize (C), serialized merge (D), clarify
  gate (E) are untouched. Only `runCard` changes: instead of nesting a workflow, it
  dispatches an orca worker and supervises by polling.

Split by nature of work:

- **Heavy / parallel / stall-prone** (Build, Review) → orca workers (separate processes).
- **Light / serial / coordination** (Linear ticket, the serialized merge, DAG
  bookkeeping, decision resolution) → in-process.

## This is already proven here

Not theoretical. In this env, right now:

- `orca status --json` → runtime `ready` / `reachable`; `orca` on PATH.
- Orchestration is enabled — `orca orchestration task-list --json` returns real tasks.
- A **completed** task (`task_2659c1d86dbc`) is exactly a ship-card worker brief:
  > isolated git worktree on branch …; read `.mex/ROUTER.md`; follow plan 024 EXACTLY;
  > VERIFY `cargo fmt --check` / `clippy -D warnings` / `cargo test --all`; GROW; SHIP
  > via `/ship` → open a PR, **do NOT merge**; report `worker_done` with the PR URL and
  > `filesModified`.

  Its `result`: `{"pr":"https://github.com/HexSleeves/inkwell/pull/34", …}`.

So the brief → worktree → `worker_done {pr}` loop has already shipped a PR by hand.
**A = put wave-ship's scheduler in charge of that loop.**

## Architecture

```
wave-ship (Workflow JS — coordinator brain, scheduler unchanged)
  Plan → DAG (B)
    └─ per READY card (deps merged, migration-serialized):
         backend=orca → runCardViaOrca(card):
            1. task-create  (spec = ship brief)
            2. worktree create --agent codex          (real worker process)
            3. dispatch --inject                       (worker reports worker_done)
            4. POLL: check --wait worker_done,escalation,decision_gate --timeout-ms 60000
                 ├ decision_gate → resolveQuestion (E) → reply
                 ├ timeout / {count:0} → log "alive", keep polling
                 └ worker_done → parse {pr,branch,cil} → status "merge-ready"
    └─ serialized merge (D) — stays IN-PROCESS (landCard → gh pr merge)
    └─ clarify gate (E)     — resolver in-process; Q/A over orca send/check
```

The supervising poller is **thin**: it only runs orca CLI and logs each poll. The
reasoning/implementation happens in the codex worker's own process — invisible to
the Workflow watchdog. The poller stays active across short `check --wait` windows
(≤60s each), so it never trips the no-progress reaper. **That is the stall fix.**

## Concept mapping

| wave-ship (now) | orca backend |
|---|---|
| `workflow(SHIP_REF, args)` per card | `task-create` + `worktree create --agent codex` + `dispatch --inject` |
| in-process subagent Build | codex CLI in its own terminal/worktree (runtime-tracked) |
| `await` the workflow result | `check --wait --types worker_done` poll loop |
| ship-card BUILD_SCHEMA return | `worker_done` payload `{taskId, dispatchId, pr, branch, cil, filesModified}` |
| clarify gate `question` (E) | worker `send --type decision_gate`; coordinator `reply` (ask verb broken → send/poll) |
| blocked retry / `MAX_REMEDIATION` | task `failed` → re-dispatch; orca circuit-breaks after 3 strikes |
| `mergedTitles` readiness | unchanged (in-process); set after the in-process merge |
| serialized merge (D) | unchanged (in-process `landCard`) |
| isolated `/tmp/ship-<cil>` worktree | orca-managed worktree; cleanup via `orca worktree remove` |

## Worker brief + protocol

Per card, wave-ship generates a self-contained brief (the proven `task_2659…` shape):

- Isolated worktree on `<branch>` off `origin/<base>` — work only here.
- Read `.mex/ROUTER.md`; follow the plan/task; match repo conventions.
- VERIFY (cargo fmt / clippy / test as the repo does); enumerate pass/fail.
- SHIP: open a PR (`/ship` or `gh pr create`), **do NOT merge** → this is *merge-ready*.
- Report `worker_done` **once** with `{taskId, dispatchId, pr, branch, cil, filesModified}`.
- **Decisions**: if blocked on a scope/approach decision, DO NOT use `ask` (broken in
  this env). `send --type decision_gate` to the coordinator with the question + options,
  then poll `check` for the reply and continue with the answer. (Maps Spike-E onto orca.)

The default `dispatch --inject` preamble tells workers to use `ask` — the brief must
**override** that with the send/poll instruction above.

## Integration seam (sketch)

```js
const BACKEND = a.backend === "orca" ? "orca" : "workflow";

async function runCard(c) {
  if (BACKEND === "orca") return runCardViaOrca(c);
  try { return { card: c, result: await workflow(SHIP_REF, cardArgs(c)), error: null }; }
  catch (e) { return { card: c, result: null, error: String(e?.message || e) }; }
}

// A THIN supervisor agent: runs orca CLI, polls, never reasons heavily.
async function runCardViaOrca(c) {
  return agent(orcaSupervisorPrompt(c, shipBrief(c)), {
    label: `orca:${c.title}`, phase: "Deploy",
    schema: BUILD_LIKE_SCHEMA, agentType: "general-purpose", effort: "low",
  })
    .then((r) => ({ card: c, result: r, error: null }))
    .catch((e) => ({ card: c, result: null, error: String(e?.message || e) }));
}
```

`orcaSupervisorPrompt` instructs the agent to: `task-create`, `worktree create
--agent codex`, `terminal wait --for tui-idle`, `dispatch --inject`, then loop
`check --wait worker_done,escalation,decision_gate --timeout-ms 60000` (log each
poll to stay active), reply to `decision_gate` via the Spike-E resolver, and on
`worker_done` return `{status:"merge-ready", pr, branch, cil, worktreePath}`.
`landCard` (D) is unchanged — it merges the PR in-process, serialized, then
`orca worktree remove` to clean up.

## Env specifics (this machine)

- `orca` at `/usr/local/bin/orca`; runtime ready; orchestration enabled.
- `ask` verb is broken → clarify gate uses `send` + poll `check` (never `ask`).
- Workers must base off `origin/<base>`, never the current feature branch (per the
  orchestration skill's worktree-base rule).
- Verify `orca status --json` at run start; bail to the `workflow` backend if the
  runtime is unreachable (headless/cron may lack it).

## Risks / unknowns

1. **Supervisor is still an in-process agent** — if IT blocks too long on a single
   `check --wait`, the watchdog could reap it. Mitigate: ≤60s windows + a log line per
   poll (the proven ship-card CI-poller trick).
2. **Linear in worker** — codex workers likely lack Linear MCP. inkwell runs
   Linear-free already; do Ticket in-process or skip (the proven brief uses `/ship`,
   no Linear).
3. **Worktree cleanup** — orca worktrees, not `/tmp`; `landCard` must `orca worktree
   remove` after merge to avoid sprawl.
4. **Concurrency** — bound dispatch with the existing `maxConcurrent`, or orca's
   `run --max-concurrent`. Avoid spawning a terminal per card unbounded.
5. **Resume/journaling** — orca task-state is durable (feeds Tier-3 G), but the
   Workflow resume cache won't replay orca side effects; resume semantics differ.
6. **Headless/cron** — interactively-authed servers may be absent; gate on `orca
   status`.

## Phased plan

- **P0 (proof, 0 new code)**: manually drive ONE inkwell card through orca —
  `task-create` → `worktree create --agent codex` → `dispatch --inject` (brief = a
  real card) → `check --wait` → confirm `worker_done {pr}` and the `ask`→send/poll
  workaround. `task_2659…` already nearly proves this.
- **P1**: `backend:"orca"` + `runCardViaOrca` supervisor; Build+Review only; `landCard`
  stays in-process. Test one card, then a 2-card file-disjoint wave.
- **P2**: clarify gate (E) over orca `send`/`check` (`decision_gate` → `reply`).
- **P3** (optional): serialized merge as an orca worker too, or keep it in-process.

**De-risk gate**: do not proceed past P1 until a stall that kills the `workflow`
backend is shown NOT to kill the `orca` backend. That is the entire justification.

## Open questions — RESOLVED for P1

1. **Worker engine → codex** for P0/P1. Add a per-card `engine` override *after* P1
   (default codex). codex is the proven path (`task_2659…` shipped PR #34); prove the
   substrate with one unknown at a time — don't introduce claude as a variable yet.
2. **Ticket / Land → in-process.** Workers lack Linear MCP, and `landCard` already
   merges serialized against a single moving base checkout (the merge-train invariant).
   Pushing the merge into a worker breaks D. Keep both coordinator-side.
3. **Granularity → one-shot worker** (full card brief = the `task_2659…` shape) for P1.
   Phase-split (separate Build/Review workers) adds coordination surface with no proven
   need; revisit only if Review needs isolation.
4. **Scheduler → keep wave-ship's JS scheduler.** Orca is the execution substrate, not
   the brain. `orca orchestration run` would discard B/C/D/E (DAG readiness, migration
   serialize, serialized merge, clarify gate) — those are the value.

**Net P1 scope:** the JS scheduler drives one-shot codex workers for Build+Review;
Ticket, serialized Land, and the clarify gate stay in-process.
