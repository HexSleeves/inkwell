export const meta = {
  name: "wave-ship",
  description:
    "Decompose a goal into a dependency DAG of cards, then ship them as a streaming fleet of autonomous ship-card runs (ticket → PR → green CI/CodeRabbit → merged → Done). Each card dispatches the moment its dependencies have merged onto the base branch, up to a global concurrency cap; migration cards serialize (≤1 unmerged at a time). Reconciles failures and auto-generates remediation/continuation cards until the goal is complete.",
  whenToUse:
    "Run a large multi-card objective end to end. Plan it into file-disjoint, independently mergeable cards with explicit dependsOn, then stream them through full ship-card autonomy: each card starts as soon as its dependencies have merged (no whole-wave barrier), bounded by maxConcurrent, with migrations serialized. Keep going (remediation + continuation) until done or a cap/budget is hit. Reusable across repos via args. Invoke at TOP LEVEL only — never nest it via workflow() from another workflow, since its parallel ship-card calls rely on being depth-1 (one-level-nesting rule).",
  phases: [
    { title: "Plan" },
    { title: "Deploy" },
    { title: "Land" },
    { title: "Reconcile" },
    { title: "Report" },
  ],
};

// ── args (all via the Workflow `args` input) ───────────────────────────────
//   repo            : absolute path to the target git repo (required)
//   goal | task     : natural-language objective to decompose into waves  ┐ one
//   plan            : path (relative to repo) of a plan file/dir to follow ┤ of
//   waves           : explicit [[card,...], ...] OR [{rationale,cards},...] ┤ these
//   cards           : flat [card,...] (layered into waves by `dependsOn`)  ┘
//     a "card" = { title, task?|plan?, scope?, labels?, priority?, dependsOn?, cil?, migration? }
//   team            : Linear team name (default "Cypress Ink Labs")
//   project         : Linear project name (default "Inkwell")
//   base            : PR base branch (default "main")
//   labels          : default Linear labels for cards that don't set their own
//   priority        : default Linear priority (1 Urgent..4 Low; default 3)
//   ignoreChecks    : CI check names ship-card treats as non-blocking
//   maxReviewRounds : ship-card CI+CodeRabbit fix-loop cap (default 5)
//   engine|engines  : ship-card Build/Review engine ("opus"|"sonnet"|"codex")
//   maxWaves        : cap on continuation rounds after the planned set (default 6)
//   maxCardsPerWave : default for maxConcurrent; planner soft target per layer (default 3)
//   maxConcurrent   : streaming DAG — global cap on cards in flight (default maxCardsPerWave)
//   maxRemediationRounds : per-card retries of BLOCKED (no-PR) cards (default 1)
//   autoContinue    : after the planned DAG drains, ask reconcile for more (default true)
//   stopOnFailedDependency : halt if cards remain whose deps can never merge (default true)
//   sequentialCards : force maxConcurrent=1 (one card in flight at a time) (default false)
//   perCardBudget   : stop dispatching when budget.remaining() drops below this (default 120000)
//   serializedMerge : wave-ship owns the merge — land PRs one-at-a-time re-checking
//                     mergeability (default true; false → ship-card self-merges)
//   answers         : map { cardTitle|cil → answer } resolving decision-gate
//                     questions on a re-run (default {})
//   plannerEngine   : model for planner/reconcile agents ("opus"|"sonnet"|"haiku"; default: inherit)
//   shipCardName    : registry name of the ship-card workflow (default "ship-card")
//   shipCardPath    : absolute path to ship-card.js (overrides shipCardName)
//   dryRun          : plan only, return the wave structure, deploy nothing (default false)
//   backend         : "workflow" (default — each card is an in-process nested
//                     ship-card run) | "orca" (Spike 003 — each card's Build+Review
//                     runs as a codex worker in an isolated Orca worktree, supervised
//                     by a thin in-process poller; execution lives in the worker's own
//                     PTY so a stalled build cannot trip the no-progress watchdog).

let a = args || {};
if (typeof a === "string") {
  try {
    a = JSON.parse(a);
  } catch (_e) {
    a = {};
  }
}

const REPO = a.repo;
const GOAL = a.goal || a.task || null;
const PLAN = a.plan || null;
const TEAM = a.team || "Cypress Ink Labs";
const PROJECT = a.project || "Inkwell";
const BASE = a.base || "main";
const DEFAULT_LABELS = Array.isArray(a.labels) ? a.labels : [];
const DEFAULT_PRIORITY = typeof a.priority === "number" ? a.priority : 3;
const IGNORE = Array.isArray(a.ignoreChecks)
  ? a.ignoreChecks
  : ["evaluate_trigger", "sandbox-verify"];
const MAX_REVIEW_ROUNDS =
  typeof a.maxReviewRounds === "number" ? a.maxReviewRounds : 5;
const MAX_WAVES = typeof a.maxWaves === "number" ? a.maxWaves : 6;
const MAX_CARDS_PER_WAVE =
  typeof a.maxCardsPerWave === "number" ? a.maxCardsPerWave : 3;
const MAX_REMEDIATION =
  typeof a.maxRemediationRounds === "number" ? a.maxRemediationRounds : 1;
const AUTO_CONTINUE = a.autoContinue !== false;
const STOP_ON_FAILED_DEP = a.stopOnFailedDependency !== false;
const SEQUENTIAL_CARDS = a.sequentialCards === true;
const MAX_CONCURRENT = SEQUENTIAL_CARDS
  ? 1
  : typeof a.maxConcurrent === "number"
    ? a.maxConcurrent
    : MAX_CARDS_PER_WAVE;
const PER_CARD_BUDGET =
  typeof a.perCardBudget === "number" ? a.perCardBudget : 120_000;
// Coordinator-owned serialized merge (Tier-2 D): when true, ship-card stops at a
// green PR (status "merge-ready", no self-merge) and wave-ship lands PRs ONE AT A
// TIME, re-checking mergeability against the moving base — so a wide layer that
// finishes together can't merge-train-race, and only this single worker touches
// the base checkout. Set false to restore ship-card self-merge.
const SERIALIZED_MERGE = a.serializedMerge !== false;
// Tier-2 E: pre-supplied answers to decision-gate questions, keyed by card title
// (or cil), fed in on a re-run to resolve questions the resolver couldn't answer.
const ANSWERS =
  a.answers && typeof a.answers === "object" && !Array.isArray(a.answers)
    ? a.answers
    : {};
const ENGINE = a.engine;
const ENGINES = a.engines;
const DRY_RUN = a.dryRun === true;
const SHIP_CARD_NAME = a.shipCardName || "ship-card";
const SHIP_CARD_PATH = a.shipCardPath || null;
const SHIP_REF = SHIP_CARD_PATH
  ? { scriptPath: SHIP_CARD_PATH }
  : SHIP_CARD_NAME;
// Spike 003 (Tier-3): card execution backend. "orca" routes runCard through a thin
// in-process supervisor that drives a codex worker in an isolated Orca worktree; the
// scheduler (DAG/migration-serialize/merge/clarify) and landCard (D) are unchanged.
const BACKEND = (() => {
  const b = a.backend;
  if (b === undefined || b === null) return "workflow";
  if (b === "orca" || b === "workflow") return b;
  // Reject typos rather than silently routing to the legacy backend.
  throw new Error(
    `wave-ship: invalid backend ${JSON.stringify(b)} (expected "workflow" or "orca")`,
  );
})();

const PLANNER_MODEL =
  a.plannerEngine && ["opus", "sonnet", "haiku"].includes(a.plannerEngine)
    ? a.plannerEngine
    : null;
function maybeModel(opts) {
  return PLANNER_MODEL ? { ...opts, model: PLANNER_MODEL } : opts;
}

if (
  !REPO ||
  (!GOAL &&
    !PLAN &&
    !(Array.isArray(a.waves) && a.waves.length) &&
    !(Array.isArray(a.cards) && a.cards.length))
) {
  log(
    "wave-ship: missing required args (need repo and one of goal|task|plan|waves|cards). Aborting.",
  );
  return {
    status: "error",
    reason: "missing required args (repo + goal|task|plan|waves|cards)",
  };
}

// ── schemas ────────────────────────────────────────────────────────────────
const CARD_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: [
    "title",
    "task",
    "plan",
    "scope",
    "labels",
    "priority",
    "dependsOn",
    "cil",
    "migration",
  ],
  properties: {
    title: {
      type: "string",
      description: "short imperative card title (also the ship-card/PR title)",
    },
    task: {
      type: ["string", "null"],
      description: "inline task for the executor; null if a plan file is used",
    },
    plan: {
      type: ["string", "null"],
      description:
        "path (relative to repo) of a plan file to follow; null if inline task",
    },
    scope: {
      type: "string",
      description: "explicit in/out-of-scope guidance for the executor",
    },
    labels: { type: "array", items: { type: "string" } },
    priority: {
      type: "number",
      description: "Linear priority 1 Urgent..4 Low",
    },
    dependsOn: {
      type: "array",
      items: { type: "string" },
      description:
        "titles of cards in EARLIER waves this relies on (informational)",
    },
    cil: {
      type: ["string", "null"],
      description:
        "existing Linear issue id to RESOLVE on retry (echo the blocked card's cil); null for a NEW card",
    },
    migration: {
      type: "boolean",
      description:
        "true if this card adds or modifies a sqlx DB migration; these serialize (≤1 unmerged at a time)",
    },
  },
};
const WAVE_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: ["index", "rationale", "cards"],
  properties: {
    index: { type: "number" },
    rationale: {
      type: "string",
      description: "why these cards belong together and at this depth",
    },
    cards: { type: "array", items: CARD_SCHEMA },
  },
};
const PLAN_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: ["ok", "goalSummary", "waves", "note"],
  properties: {
    ok: { type: "boolean" },
    goalSummary: { type: "string" },
    waves: { type: "array", items: WAVE_SCHEMA },
    note: { type: "string" },
  },
};
const RECONCILE_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: ["goalComplete", "needNewWave", "newWaveCards", "blockers", "note"],
  properties: {
    goalComplete: {
      type: "boolean",
      description:
        "true only when the objective is fully delivered by merged cards",
    },
    needNewWave: { type: "boolean" },
    newWaveCards: { type: "array", items: CARD_SCHEMA },
    blockers: {
      type: "array",
      items: { type: "string" },
      description: "items needing human attention; one line each",
    },
    note: { type: "string" },
  },
};

// ── Tier-2 E: decision-gate resolver schema ──────────────────────────────────
const RESOLVE_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: ["answered", "answer", "confidence", "rationale"],
  properties: {
    answered: {
      type: "boolean",
      description:
        "true ONLY if the question is answerable confidently from the objective/repo conventions (not a product/risk judgment a human must make)",
    },
    answer: { type: ["string", "null"] },
    confidence: { type: "string", enum: ["high", "medium", "low"] },
    rationale: { type: "string" },
  },
};

// ── card helpers ─────────────────────────────────────────────────────────────
function normCard(c, i) {
  const idx = typeof i === "number" ? i : 0;
  return {
    title: c.title || `card-${idx + 1}`,
    task: c.task || (c.plan ? null : c.title || null),
    plan: c.plan || null,
    scope:
      c.scope ||
      (c.plan
        ? "Follow the plan's Scope section exactly."
        : "Keep the change minimal and focused on the task."),
    labels: Array.isArray(c.labels) ? c.labels : DEFAULT_LABELS,
    priority: typeof c.priority === "number" ? c.priority : DEFAULT_PRIORITY,
    dependsOn: Array.isArray(c.dependsOn) ? c.dependsOn : [],
    cil: c.cil || null,
    migration: c.migration === true,
  };
}

// Translate one wave card into ship-card's arg contract.
function cardArgs(c) {
  return {
    repo: REPO,
    title: c.title,
    plan: c.plan || undefined,
    task: c.plan ? undefined : c.task || c.title,
    scope: c.scope,
    cil: c.cil || undefined,
    team: TEAM,
    project: PROJECT,
    base: BASE,
    labels: c.labels,
    priority: c.priority,
    ignoreChecks: IGNORE,
    maxReviewRounds: MAX_REVIEW_ROUNDS,
    engine: ENGINE,
    engines: ENGINES,
    // Tier-2 D: hand the green PR back for serialized coordinator merge (or self).
    land: SERIALIZED_MERGE ? "coordinator" : "self",
  };
}

// Run ONE card through the configured execution backend. Both return the SAME
// shape ({card, result:{status,...}, error}); the scheduler routes on status, so
// "orca" is invisible to merge (D), retry, and DAG bookkeeping.
async function runCard(c) {
  if (BACKEND === "orca") {
    const out = await runCardViaOrca(c);
    // Spike 003: if the Orca runtime is unreachable (e.g. headless/cron), fall back
    // to the in-process workflow backend rather than failing the card. Any other
    // orca error is a real failure and is returned as-is.
    if (out.error !== "orca runtime unreachable") return out;
    log(
      `wave-ship: orca runtime unreachable for "${c.title}" → workflow-backend fallback.`,
    );
  }
  try {
    const res = await workflow(SHIP_REF, cardArgs(c));
    return { card: c, result: res, error: null };
  } catch (e) {
    return { card: c, result: null, error: String((e && e.message) || e) };
  }
}

// ── Spike 003: orca execution backend ────────────────────────────────────────
// Each card's heavy work (Build + Review → green PR) runs as a codex worker in an
// isolated Orca worktree. A THIN in-process supervisor agent only runs `orca` CLI
// and polls `check --wait` in ≤60s windows (logging each, so it never idles) — the
// reasoning/implementation happens in the worker's own PTY, invisible to the
// Workflow no-progress watchdog. On success it returns status "merge-ready" so the
// existing serialized landCard (D) merges it in-process, then cleans up the worktree.
const ORCA_SCHEMA = {
  type: "object",
  additionalProperties: false,
  // Only status + note are required: error/blocked/timeout early-returns legitimately
  // omit pr/branch/etc, and forcing them would make the schema reject a valid stop.
  required: ["status", "note"],
  properties: {
    status: {
      type: "string",
      description:
        'one of: "merge-ready" (green PR opened, not merged), "blocked" (no PR; safe to auto-retry), "error" (a PR may exist or the runtime failed; needs a human)',
    },
    pr: { type: ["number", "null"], description: "PR number, or null" },
    prUrl: { type: ["string", "null"] },
    branch: { type: ["string", "null"] },
    cil: { type: ["string", "null"] },
    worktreeSelector: {
      type: ["string", "null"],
      description: 'Orca worktree selector for cleanup, e.g. "name:ship-foo"',
    },
    note: { type: "string" },
    // Decision-gate fields (status "blocked") — preserved so the unchanged
    // clarify handling (E) can surface the question + options to a human.
    blockReason: { type: ["string", "null"] },
    question: {
      type: ["string", "null"],
      description: "decision-gate question when status is blocked",
    },
    questionOptions: { type: "array", items: { type: "string" } },
  },
};

// Deterministic Orca worktree name from the card title (no Math.random in scripts).
function wtName(c) {
  const s =
    String(c.title || "card")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 28) || "card";
  // Deterministic suffix from stable card identity (no Math.random in scripts) so
  // cards whose titles collide after slug/truncation still get distinct worktrees.
  const seed = `${c.title || ""}|${c.cil || ""}|${c.plan || ""}|${c.task || ""}`;
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return `ship-${s}-${h.toString(36).slice(0, 6)}`;
}

// The self-contained ship brief handed to the codex worker (the proven task_2659…
// shape): isolate, follow the task/plan, VERIFY, open a PR, do NOT merge, report once.
function orcaBrief(c) {
  const work = c.plan
    ? `Follow the plan file ${c.plan} EXACTLY.`
    : `Implement this task:\n${c.task || c.title}`;
  return [
    `CARD: ${c.title}`,
    `You are in an isolated Orca git worktree based off origin/${BASE}. Work ONLY in this worktree. Work FULLY AUTONOMOUSLY; do NOT ask interactive questions.`,
    ``,
    `Read .mex/ROUTER.md for repo conventions before editing. ${work}`,
    `Scope: ${c.scope || "Keep the change minimal and focused on the task."}`,
    ``,
    `VERIFY exactly as this repo's CI does (run all that apply): \`cargo fmt --all -- --check\`, \`cargo clippy --all-targets --all-features --locked -- -D warnings\`, \`cargo test --all --locked\`. Enumerate each pass/fail. If any required check fails, FIX it before opening a PR.`,
    `SHIP: commit on this worktree's branch with a conventional-commit message, then open a PR with \`gh pr create --base ${BASE}\` (clear title + body). Do NOT merge — leave it merge-ready.`,
    `REPORT: send EXACTLY ONE worker_done message to the coordinator with a JSON body {taskId, dispatchId, pr, prUrl, branch, cil, filesModified}. pr = the PR number; prUrl = its URL.`,
    `DECISIONS: do NOT use the \`ask\` verb (broken here). If you must ask, \`orca orchestration send --type decision_gate\` to the coordinator with the question + options, then poll \`orca orchestration check --wait\` for the reply and continue.`,
    c.cil
      ? `Linear ticket for this card: ${c.cil} (resolve it; do not create a new one).`
      : `No Linear ticket — skip Linear.`,
  ].join("\n");
}

// Prompt for the thin supervisor agent: drive the proven orca CLI sequence + poll.
function orcaSupervisorPrompt(c) {
  const name = wtName(c);
  const brief = orcaBrief(c);
  return `You are the wave-ship ORCA SUPERVISOR for ONE card. You are THIN: you ONLY run \`orca\` CLI commands and poll — you do NOT implement the card yourself (a codex worker does, in its own worktree). Work autonomously; never block on a human.

CARD TITLE: ${c.title}
REPO: ${REPO}    BASE: ${BASE}    WORKTREE NAME: ${name}

Run each step with \`--json\` and read the fields named:

1. PREFLIGHT: \`orca status --json\`. If result.runtime.state != "ready" or not reachable → STOP, return {status:"error", note:"orca runtime unreachable"}.
2. TASK: \`orca orchestration task-create --spec <BRIEF> --task-title ${JSON.stringify(c.title)} --json\` (BRIEF is verbatim below). Capture result.task.id (TASK_ID) and result.task.created_by_terminal_handle (COORDINATOR handle).
3. WORKER: \`orca worktree create --name ${name} --repo path:${REPO} --agent codex --base-branch origin/${BASE} --no-parent --json\`. Capture result.startupTerminal.handle (WORKER handle) and result.worktree.branch.
4. WAIT: \`orca terminal wait --terminal <WORKER> --for tui-idle --timeout-ms 90000\`, then \`orca terminal read --terminal <WORKER>\` once to confirm boot (a hooks-trust prompt may auto-resolve; a \`railway\` MCP auth failure is harmless).
5. DISPATCH: \`orca orchestration dispatch --task <TASK_ID> --to <WORKER> --from <COORDINATOR> --inject --json\`. codex auto-submits the injected brief. If a follow-up \`terminal read\` shows the brief unsent in the composer, run \`orca terminal send --terminal <WORKER> --text "\\n"\` once.
6. SUPERVISE — loop up to ~20 windows (~20 min): \`orca orchestration check --terminal <COORDINATOR> --wait --types worker_done,escalation,decision_gate --timeout-ms 60000 --json\`. Emit one short log line after EACH call so you stay active.
   - worker_done → parse its JSON body → return {status:"merge-ready", pr:<number>, prUrl:<url>, branch:<branch>, cil:${c.cil ? JSON.stringify(c.cil) : "null"}, worktreeSelector:"name:${name}", note:"opened PR"}.
   - decision_gate → if the answer is clear from repo conventions / the task, \`orca orchestration reply --id <msgId> --body <answer>\` and keep polling; if it needs a human product/scope call, STOP: first run \`orca worktree rm --worktree name:${name} --force --json\` to clean up so a future retry does not collide, then return {status:"blocked", question:"<the question>", questionOptions:[<options as strings>], blockReason:"<short why blocked>", note:"<the question>", cil:${c.cil ? JSON.stringify(c.cil) : "null"}, worktreeSelector:"name:${name}"}.
   - escalation → STOP, return {status:"error", note:"<escalation text>", worktreeSelector:"name:${name}"}.
   - no worker_done after the windows → return {status:"error", note:"worker timeout (no worker_done in ~20m)", worktreeSelector:"name:${name}"}.

Do NOT merge the PR and do NOT remove the worktree — the coordinator lands and cleans up. Return ONLY the structured result.

BRIEF (pass verbatim as --spec in step 2):
"""
${brief}
"""`;
}

// Supervise ONE card via orca; map the supervisor result into the scheduler shape.
async function runCardViaOrca(c) {
  try {
    const r = await agent(
      orcaSupervisorPrompt(c),
      maybeModel({
        label: `orca:${c.title}`,
        phase: "Deploy",
        schema: ORCA_SCHEMA,
        agentType: "general-purpose",
        effort: "low",
      }),
    );
    const rawStatus =
      r?.status === "merge-ready" || r?.status === "blocked"
        ? r.status
        : "error";
    const missingMergeCoords =
      rawStatus === "merge-ready" && (!r?.pr || !r?.prUrl || !r?.branch);
    const s = missingMergeCoords ? "error" : rawStatus;
    return {
      card: c,
      result: {
        status: s,
        cil: r?.cil ?? c.cil ?? null,
        pr: r?.pr ?? null,
        prUrl: r?.prUrl ?? null,
        branch: r?.branch ?? null,
        worktreeSelector: r?.worktreeSelector || `name:${wtName(c)}`,
        backend: "orca",
        // Carry the ship-card "blocked" contract: clarify (E) reads detail.question/
        // questionOptions, and isRetriable()'s dup-PR guard checks detail.prUrl/
        // prNumber — so a "blocked" that somehow has a PR is NOT auto-retried.
        detail: {
          note: r?.note || "",
          prUrl: r?.prUrl ?? null,
          prNumber: r?.pr ?? null,
          question: r?.question ?? null,
          questionOptions: Array.isArray(r?.questionOptions)
            ? r.questionOptions
            : [],
          blockReason:
            r?.blockReason ?? (s === "blocked" ? (r?.note ?? null) : null),
        },
      },
      error: s === "error"
        ? missingMergeCoords
          ? "orca merge-ready result missing pr/prUrl/branch"
          : r?.note || "orca worker error"
        : null,
    };
  } catch (e) {
    return { card: c, result: null, error: String((e && e.message) || e) };
  }
}

// ── Tier-2 D: serialized coordinator merge ───────────────────────────────────
// With SERIALIZED_MERGE on, ship-card returns a green PR without merging
// ("merge-ready"); wave-ship lands those PRs one-at-a-time via landCard so the
// base advances serially and migrations/CI can't race at the merge boundary.
const LAND_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: ["merged", "mergeSha", "ticketDone", "note"],
  properties: {
    merged: { type: "boolean" },
    mergeSha: { type: ["string", "null"] },
    ticketDone: { type: "boolean" },
    note: { type: "string" },
  },
};

function landPrompt(rc) {
  const r = rc.result || {};
  const ignoreLine = IGNORE.join(", ");
  const ticketStep = r.cil
    ? `Load tools: ToolSearch "select:mcp__plugin_linear_linear__save_issue,mcp__plugin_linear_linear__save_comment". save_issue { id: "${r.cil}", state: "Done" }, then save_comment on ${r.cil} recapping: "Landed on ${BASE} — squash-merged PR #${r.pr} as <sha>." If Linear is unavailable, still report merged=true with ticketDone=false and a note.`
    : `No ticket id — skip Linear and set ticketDone=true.`;
  // Spike 003: orca-managed worktrees are torn down with `orca worktree rm` (removes
  // worktree + branch from Orca and git); workflow-backend worktrees use plain git.
  const cleanupStep =
    r.backend === "orca" && r.worktreeSelector
      ? `Remove the Orca worktree (this also deletes its branch): orca worktree rm --worktree ${r.worktreeSelector} --force ; orca worktree prune (ignore errors).`
      : `Remove the worktree if present: git worktree remove --force ${r.worktreePath || ""} ; git worktree prune. Delete the stale local branch: git branch -D ${r.branch} (ignore errors).`;
  return `You are the wave-ship MERGE coordinator landing ONE already-green PR. Work autonomously; do not ask questions.
PR #${r.pr} on the repo at ${REPO} — branch ${r.branch}, base ${BASE}, ticket ${r.cil || "(none)"}, worktree ${r.worktreePath || "(none)"}.
This PR already passed CI + CodeRabbit in its ship-card run. You OWN the merge so sibling merges land serially, each on the CURRENT base.

== Pre-flight (re-check — the base may have moved since this PR went green) ==
cd ${REPO}; git fetch origin. Inspect: gh pr view ${r.pr} --json state,mergeable,mergeStateStatus  and  gh pr checks ${r.pr}.
- If state is already MERGED → idempotent success: return merged=true with the existing squash SHA from \`gh pr view ${r.pr} --json mergeCommit --jq .mergeCommit.oid\` (no checkout).
- Treat these check names as non-blocking: ${ignoreLine}. Also ignore "skipping"/"neutral". Every OTHER check must be "pass".
- If mergeable is CONFLICTING, or a required check regressed/failed → DO NOT force and DO NOT rebase+re-wait here (re-running CI would stall the serialized merge queue). Return merged=false with a one-line note naming the conflict/check so a human can rebase ${r.branch} and re-land.

== Merge (squash) ==
gh pr merge ${r.pr} --squash --delete-branch. Capture the squash commit SHA WITHOUT touching any working tree: \`gh pr view ${r.pr} --json mergeCommit --jq .mergeCommit.oid\`. Do NOT run \`git checkout\` or \`git pull\` in ${REPO} — wave-ship may be running from the user's working checkout and must never switch its branch. GitHub advances origin/${BASE} on merge, so sibling workers (which base off origin/${BASE}) still see the new base.

== Close ticket + cleanup ==
${ticketStep}
${cleanupStep}

== Hard rules ==
Never fabricate a merge. Merge only this one PR. Return the structured result (merged, mergeSha, ticketDone, note).`;
}

// Land ONE merge-ready card. The caller serializes these (single in-flight slot).
async function landCard(rc) {
  try {
    const land = await agent(
      landPrompt(rc),
      maybeModel({
        label: `land:${rc.result?.cil || rc.card.title}`,
        phase: "Land",
        schema: LAND_SCHEMA,
        agentType: "general-purpose",
      }),
    );
    return {
      merged: !!land?.merged,
      mergeSha: land?.mergeSha || null,
      ticketDone: !!land?.ticketDone,
      note: land?.note || "",
    };
  } catch (e) {
    return {
      merged: false,
      mergeSha: null,
      ticketDone: false,
      note: String((e && e.message) || e),
    };
  }
}

function statusOf(r) {
  return r?.result?.status || (r?.error ? "error" : "unknown");
}
const isMerged = (r) => statusOf(r) === "merged";
// Only BLOCKED cards are safe to auto-retry: they stopped BEFORE opening a PR,
// so a re-run won't create a duplicate PR. needs-attention / merge-failed have
// an open PR and must go to a human. The extra detail.pr* guard defends against
// a Build agent that set blocked=true yet still opened a PR (contract violation).
const isRetriable = (r) =>
  statusOf(r) === "blocked" &&
  !r.result?.detail?.prUrl &&
  !r.result?.detail?.prNumber;

// Pure-JS topological layering for a flat `cards` list (no planner agent needed).
function layerByDeps(rawCards) {
  const norm = rawCards.map((c, i) => normCard(c, i));
  const byTitle = new Map(norm.map((c) => [c.title, c]));
  const layerOf = new Map();
  const visiting = new Set();
  let cycle = false;
  function depth(c) {
    if (layerOf.has(c.title)) return layerOf.get(c.title);
    if (visiting.has(c.title)) {
      cycle = true; // back-edge: dependency cycle (the visiting set stops recursion)
      layerOf.set(c.title, 0);
      return 0;
    }
    visiting.add(c.title);
    let d = 0;
    for (const dep of c.dependsOn) {
      const dc = byTitle.get(dep);
      if (dc && dc !== c) d = Math.max(d, depth(dc) + 1);
    }
    visiting.delete(c.title);
    layerOf.set(c.title, d);
    return d;
  }
  norm.forEach(depth);
  if (cycle) {
    // Garbage-in: a cycle would co-locate mutually-dependent cards in one
    // parallel wave (merge-conflict risk). Serialize one card per wave instead.
    log(
      "wave-ship: dependency CYCLE detected in cards — serializing one card per wave.",
    );
    return norm.map((c, i) => ({
      index: i + 1,
      rationale: "serialized (dependency cycle detected)",
      cards: [c],
    }));
  }
  const maxD = norm.reduce((m, c) => Math.max(m, layerOf.get(c.title)), 0);
  const waves = [];
  for (let d = 0; d <= maxD; d++) {
    const cs = norm.filter((c) => layerOf.get(c.title) === d);
    if (cs.length)
      waves.push({
        index: waves.length + 1,
        rationale: `dependency layer ${d}`,
        cards: cs,
      });
  }
  return waves;
}

function wavesFromExplicit(raw) {
  return raw.map((w, wi) => {
    const cards = Array.isArray(w) ? w : Array.isArray(w.cards) ? w.cards : [];
    return {
      index: wi + 1,
      rationale: (!Array.isArray(w) && w.rationale) || "explicit wave",
      cards: cards.map((c, i) => normCard(c, i)),
    };
  });
}

function toFailure(r) {
  return {
    title: r.card.title,
    status: statusOf(r),
    cil: r.result?.cil || null,
    pr: r.result?.pr || null,
    prUrl: r.result?.prUrl || null,
    note:
      r.result?.detail?.blockReason ||
      r.result?.detail?.summary ||
      r.result?.note ||
      r.error ||
      "",
  };
}
function toOutcome(r) {
  return {
    title: r.card.title,
    status: statusOf(r),
    cil: r.result?.cil || null,
    pr: r.result?.pr || null,
    mergeSha: r.result?.mergeSha || null,
  };
}

// ── reconcile prompts ────────────────────────────────────────────────────────
function remediatePrompt(ctx) {
  return `You are the wave-ship RECONCILE agent in REMEDIATE mode for ${ctx.waveLabel} on the repo at ${REPO}.
Each card below was shipped with the autonomous ship-card flow. Outcomes (JSON):
${JSON.stringify(ctx.failures, null, 2)}

Decide which failed cards can be SAFELY retried, and how.
RULES:
- ONLY status="blocked" cards are retriable — they stopped BEFORE opening a PR, so a re-run cannot create a duplicate PR. For each, emit a refined retry card in newWaveCards: keep the SAME title, set cil to that card's cil so its existing Linear ticket resolves, and sharpen task/scope using the block note so the executor can get past the blocker. Set dependsOn to []. Echo \`migration\` from the blocked card unchanged.
- status="needs-attention" / "merge-failed" / "error" cards already have an open PR or a non-retriable failure. Do NOT propose auto-retry (it would duplicate the PR). Put each in blockers with a one-line human action.
- status="merged" cards are done — ignore. status="skipped-budget" — ignore.
Set needNewWave=true iff newWaveCards is non-empty. goalComplete=false. Return the structured result.`;
}

function continuePrompt(ctx) {
  return `You are the wave-ship RECONCILE agent in CONTINUE mode for an objective on the repo at ${REPO} (base ${BASE}). Work autonomously; inspect the repo if needed.
OBJECTIVE:
${ctx.goal}

Cards shipped so far (JSON):
${JSON.stringify(ctx.results, null, 2)}

Judge whether the OBJECTIVE is fully delivered by the MERGED cards.
- Complete → goalComplete=true, needNewWave=false, newWaveCards=[].
- Not complete AND clear remaining in-scope work exists → goalComplete=false, needNewWave=true, propose newWaveCards for the NEXT wave. Those cards MUST be runnable IN PARALLEL: file-disjoint, each independently mergeable onto ${BASE} (which now contains every merged card). At most ONE may add/modify a sqlx migration (migrations serialize) — set \`migration\`:true on it and put any second migration in a later continuation; set \`migration\`:false on the rest. Each needs a precise self-contained task + scope; an autonomous executor implements it with no further questions. dependsOn lists already-merged card titles if relevant. Set each new card's cil to null.
- Remaining work blocked on human input or unclear scope → goalComplete=false, needNewWave=false, list it in blockers.
Return the structured result.`;
}

async function reconcile(mode, ctx) {
  const prompt =
    mode === "remediate" ? remediatePrompt(ctx) : continuePrompt(ctx);
  return agent(
    prompt,
    maybeModel({
      label: `reconcile:${mode}`,
      phase: "Reconcile",
      schema: RECONCILE_SCHEMA,
      agentType: "general-purpose",
      effort: "high",
    }),
  );
}

// ── Tier-2 E: decision-gate resolver ─────────────────────────────────────────
function resolverPrompt(card, question, options) {
  return `You are the wave-ship DECISION RESOLVER. A card's executor is blocked on ONE ambiguous decision and needs an answer to proceed. Answer ONLY if it is confidently derivable from the OBJECTIVE or the repo's existing conventions — never invent a product/UX/risk judgment that a human owner must make.

OBJECTIVE:
${GOAL || PLAN}

CARD: ${card.title}
SCOPE: ${card.scope}
QUESTION: ${question}
${options && options.length ? `CANDIDATE ANSWERS: ${options.map((o, i) => `(${i + 1}) ${o}`).join("  ")}` : "CANDIDATE ANSWERS: (none provided)"}

Inspect the repo if needed (conventions, existing patterns, the plan/objective). If the answer is clear from those, set answered=true with a concrete, actionable answer (pick one option or state the decision) and confidence high|medium. If it requires a human's product/scope/risk call, set answered=false (the coordinator escalates to a human). Return the structured result.`;
}

async function resolveQuestion(card, question, options) {
  // 1. Pre-supplied answer (resume path): keyed by card title or its Linear id.
  const pre = ANSWERS[card.title] || (card.cil && ANSWERS[card.cil]) || null;
  if (pre) return { answer: String(pre), source: "supplied" };
  // 2. Auto-resolve from the objective / repo conventions.
  const res = await agent(
    resolverPrompt(card, question, options),
    maybeModel({
      label: `resolve:${card.title}`,
      phase: "Reconcile",
      schema: RESOLVE_SCHEMA,
      agentType: "general-purpose",
      effort: "high",
    }),
  );
  if (res?.answered && res.answer && res.confidence !== "low")
    return { answer: res.answer, source: "resolver" };
  return null;
}

// ── Phase 1: plan the waves ──────────────────────────────────────────────────
phase("Plan");
let plannedWaves;
if (Array.isArray(a.waves) && a.waves.length) {
  plannedWaves = wavesFromExplicit(a.waves);
  log(`wave-ship: using ${plannedWaves.length} explicit wave(s) from args.`);
} else if (Array.isArray(a.cards) && a.cards.length) {
  plannedWaves = layerByDeps(a.cards);
  log(
    `wave-ship: layered ${a.cards.length} explicit card(s) into ${plannedWaves.length} wave(s) by dependency.`,
  );
} else {
  const plannerPrompt = `You are the wave PLANNER for an autonomous shipping pipeline on the git repo at ${REPO} (base branch ${BASE}). Work autonomously; inspect the repo as needed.

OBJECTIVE:
${PLAN ? `Follow the plan file/dir \`${PLAN}\` (read it IN FULL).` : GOAL}

Decompose the objective into an ORDERED list of WAVES of CARDS. Each card is one PR-sized unit that the ship-card flow lands as a single squash commit.

HARD INVARIANTS:
1. Cards in the SAME wave run IN PARALLEL and each merges independently onto ${BASE}. They MUST be file-disjoint: no two same-wave cards edit the same files, share a DB migration, or depend on each other's code. If two pieces overlap or one needs the other, put them in DIFFERENT waves.
2. Wave order = dependency depth. Wave 1 has no dependencies. Each later wave may depend ONLY on earlier waves, which are fully MERGED onto ${BASE} before the later wave starts (so its cards branch off the updated base and can see that code).
3. Each card is self-contained: a precise \`task\` (what to build) and \`scope\` (explicit in/out of scope) so an autonomous executor needs no clarification. If the repo has a plan doc for a piece, reference it via the card's \`plan\` field (path relative to repo) and leave \`task\` null.
4. Keep cards coherent and PR-sized. Split anything spanning unrelated areas; fold trivially-small work into a sibling. Aim for at most ${MAX_CARDS_PER_WAVE} cards per wave where natural (more is allowed; they batch).
5. Per card set labels/priority (default labels ${JSON.stringify(DEFAULT_LABELS)}, default priority ${DEFAULT_PRIORITY}). dependsOn = titles of EARLIER-wave cards it relies on.
6. Every card is NEW work — set its \`cil\` to null (ship-card creates the ticket).
7. DB migrations SERIALIZE. At most ONE card per wave may add or modify a sqlx migration — Railway auto-migrates on each merge and sqlx rejects out-of-order migrations, so two unmerged migration PRs race and corrupt the sequence. Put any second migration in a LATER wave. Set \`migration\`:true on every card that adds/edits a migration and false on all others.

Number waves from 1. ok=true with goalSummary, the waves array, and a note. If the objective is too vague to plan safely, ok=false explaining what's missing. Return the structured result.`;

  const plan = await agent(
    plannerPrompt,
    maybeModel({
      label: "plan",
      phase: "Plan",
      schema: PLAN_SCHEMA,
      agentType: "general-purpose",
      effort: "high",
    }),
  );
  if (!plan?.ok || !Array.isArray(plan.waves) || !plan.waves.length) {
    log(`wave-ship: planning failed — ${plan?.note || "no waves produced"}`);
    return { status: "error", phase: "Plan", detail: plan };
  }
  plannedWaves = plan.waves.map((w, i) => ({
    index: w.index || i + 1,
    rationale: w.rationale || "",
    cards: (w.cards || []).map((c, ci) => normCard(c, ci)),
  }));
  log(
    `wave-ship: planned ${plannedWaves.length} wave(s), ${plannedWaves.reduce((n, w) => n + w.cards.length, 0)} card(s) total.`,
  );
}

if (DRY_RUN) {
  log("wave-ship: dryRun — returning the wave plan without deploying.");
  return {
    status: "planned",
    dryRun: true,
    objective: GOAL || PLAN,
    waves: plannedWaves.map((w) => ({
      index: w.index,
      rationale: w.rationale,
      cards: w.cards.map((c) => ({
        title: c.title,
        scope: c.scope,
        plan: c.plan,
        task: c.task,
        dependsOn: c.dependsOn,
        migration: c.migration,
      })),
    })),
  };
}

// ── Phase 2+3: deploy cards as a STREAMING DAG (remediation + continuation) ───
// A card dispatches the moment its `dependsOn` are ALL merged onto BASE, up to
// MAX_CONCURRENT in flight, re-evaluated on every completion — so no card waits
// past its real dependencies (vs the old whole-wave barrier). Migration cards are
// mutually exclusive in flight (≤1 unmerged at a time) to avoid sqlx out-of-order
// numbering + Railway per-merge auto-migrate races. The planner's wave numbering
// survives only as a layering hint (`_wave`), never as an execution gate.
const allResults = [];
const reconcileBlockers = [];
const decisionGates = []; // unresolved decision-gate questions for a human
const mergedTitles = new Set();
const failedTitles = new Set(); // terminally not-merged → can never satisfy a dep
const remCount = new Map(); // card title → remediation attempts already spent
const byTitle = new Map(); // card title → card object (migration + retry lookups)
const inflight = new Map(); // card title → Promise<{ kind:"card", title, out }>
const pool = []; // cards awaiting dispatch (deps not yet all merged)
const mergeQueue = []; // green merge-ready cards awaiting serialized land
let merging = null; // the single in-flight land Promise (serializes merges)
let continuations = 0;
let stopped = null;
let goalComplete = false;

for (const w of plannedWaves)
  for (const c of w.cards) {
    c._wave = w.index;
    if (byTitle.has(c.title))
      log(
        `wave-ship: duplicate card title "${c.title}" — dependencies key on title; later card wins.`,
      );
    byTitle.set(c.title, c);
    pool.push(c);
  }
const totalPlanned = pool.length;
log(
  `wave-ship: streaming ${totalPlanned} card(s) from ${plannedWaves.length} planned layer(s); maxConcurrent=${MAX_CONCURRENT}.`,
);

const budgetLow = () =>
  !!(budget?.total && budget.remaining() < PER_CARD_BUDGET);
const depsMerged = (c) => c.dependsOn.every((d) => mergedTitles.has(d));
const titlesInPlay = () =>
  new Set([...pool.map((c) => c.title), ...inflight.keys()]);
const migrationInFlight = () => {
  for (const t of inflight.keys()) {
    const c = byTitle.get(t);
    if (c && c.migration) return true;
  }
  return false;
};
function pushHeld(c, reason) {
  failedTitles.add(c.title);
  allResults.push({
    wave: c._wave || 0,
    card: c,
    result: { status: "held-dependency" },
    error: reason,
  });
}

while (true) {
  // 1. Retire cards whose deps can NEVER merge (a dep failed and is gone from
  //    play). Their own dependents cascade-fail on a later pass via this rule.
  const inPlay = titlesInPlay();
  for (const c of [...pool]) {
    const deadDep = c.dependsOn.find(
      (d) => failedTitles.has(d) && !inPlay.has(d),
    );
    if (deadDep) {
      pool.splice(pool.indexOf(c), 1);
      log(`wave-ship: dropping "${c.title}" — dependency "${deadDep}" failed.`);
      pushHeld(c, `failed dependency: ${deadDep}`);
    }
  }

  // 2. Pick a dispatchable set: deps merged, within MAX_CONCURRENT, and at most
  //    ONE migration card unmerged at a time (serialize migrations).
  const low = budgetLow();
  const migBusy = migrationInFlight();
  const dispatch = [];
  if (!low) {
    for (const c of pool) {
      if (inflight.size + dispatch.length >= MAX_CONCURRENT) break;
      if (!depsMerged(c)) continue;
      if (c.migration && (migBusy || dispatch.some((d) => d.migration)))
        continue;
      dispatch.push(c);
    }
  }
  if (dispatch.length) {
    phase("Deploy");
    for (const c of dispatch) {
      pool.splice(pool.indexOf(c), 1);
      log(
        `wave-ship: dispatch "${c.title}"${c.migration ? " [migration]" : ""} (layer ${c._wave || "?"}; ${inflight.size + 1} in flight).`,
      );
      const p = runCard(c).then((out) => ({ kind: "card", title: c.title, out }));
      inflight.set(c.title, p);
    }
  }

  // 2b. Serialized merge: if a green PR is queued and no merge is running, land
  //     exactly ONE. Each merge advances the base alone so siblings can't race.
  //     Drain whenever the queue is non-empty (anything queued was handed back
  //     specifically to be merged here) — not gated on SERIALIZED_MERGE.
  if (!merging && mergeQueue.length) {
    const rc = mergeQueue.shift();
    phase("Land");
    log(
      `wave-ship: merging "${rc.card.title}" (PR #${rc.result?.pr}; ${mergeQueue.length} more queued).`,
    );
    merging = landCard(rc).then((landed) => ({
      kind: "merge",
      title: rc.card.title,
      rc,
      landed,
    }));
  }

  // 3. Nothing running → the pool drained (try a continuation wave) or we are
  //    stuck on unsatisfiable deps / budget.
  if (inflight.size === 0 && !merging && mergeQueue.length === 0) {
    if (pool.length === 0) {
      if (!AUTO_CONTINUE || stopped || budgetLow()) {
        if (!stopped && budgetLow()) stopped = "budget";
        break;
      }
      if (continuations >= MAX_WAVES) {
        if (!goalComplete) stopped = "max-waves";
        break;
      }
      continuations++;
      phase("Reconcile");
      const cont = await reconcile("continue", {
        goal: GOAL || PLAN,
        results: allResults.map(toOutcome),
      });
      if (Array.isArray(cont?.blockers))
        reconcileBlockers.push(...cont.blockers);
      if (cont?.goalComplete) {
        goalComplete = true;
        log("wave-ship: reconcile reports the goal is COMPLETE.");
        break;
      }
      const more = (cont?.newWaveCards || []).map((c, i) => normCard(c, i));
      if (!more.length) {
        log("wave-ship: no continuation cards proposed; finishing.");
        break;
      }
      let maxWave = 0;
      for (const c of byTitle.values())
        maxWave = Math.max(maxWave, c._wave || 0);
      for (const c of more) {
        c._wave = maxWave + 1;
        byTitle.set(c.title, c);
        pool.push(c);
      }
      log(
        `wave-ship: continuation layer (${more.length} card(s)): ${more.map((c) => c.title).join(" | ")}`,
      );
      continue;
    }
    // pool non-empty, nothing in flight, nothing dispatchable → unsatisfiable.
    if (budgetLow()) {
      stopped = "budget";
    } else if (STOP_ON_FAILED_DEP) {
      stopped = "failed-dependency";
    }
    const heldCount = pool.length;
    for (const c of [...pool]) {
      const missing = c.dependsOn.filter((d) => !mergedTitles.has(d));
      pushHeld(c, `unmet deps: ${missing.join(", ")}`);
    }
    pool.length = 0;
    log(
      `wave-ship: stopping (${stopped || "drained"}) — ${heldCount} card(s) held on unmet dependencies.`,
    );
    break;
  }

  // 4. Block until ONE in-flight thing finishes — a card's build/review run, or
  //    the single serialized merge — then re-evaluate readiness.
  const ev = await Promise.race([
    ...inflight.values(),
    ...(merging ? [merging] : []),
  ]);

  if (ev.kind === "merge") {
    merging = null;
    const mt = ev.title;
    const mcard = byTitle.get(mt) || ev.rc.card;
    const r = ev.rc.result || {};
    if (ev.landed.merged) {
      mergedTitles.add(mt);
      allResults.push({
        wave: mcard._wave || 0,
        card: mcard,
        result: {
          status: "merged",
          cil: r.cil || null,
          pr: r.pr || null,
          prUrl: r.prUrl || null,
          mergeSha: ev.landed.mergeSha,
          ticketDone: ev.landed.ticketDone,
        },
        error: null,
      });
      log(`wave-ship: MERGED "${mt}" (${mergedTitles.size} merged so far).`);
    } else {
      // Green PR that could not be landed (conflict / regressed check) → human.
      failedTitles.add(mt);
      allResults.push({
        wave: mcard._wave || 0,
        card: mcard,
        result: {
          status: "merge-failed",
          cil: r.cil || null,
          pr: r.pr || null,
          prUrl: r.prUrl || null,
          note: ev.landed.note,
        },
        error: ev.landed.note,
      });
      log(`wave-ship: merge BLOCKED for "${mt}" — ${ev.landed.note}`);
    }
    continue;
  }

  // ev.kind === "card": a build/review run finished.
  const { title, out } = ev;
  inflight.delete(title);
  const card = byTitle.get(title) || out.card;
  const status = statusOf(out);

  if (status === "merge-ready") {
    // Green PR handed back — queue for serialized coordinator merge. NOT merged
    // yet, so it does not satisfy dependents until landCard lands it.
    mergeQueue.push(out);
    log(`wave-ship: "${title}" green → queued for serialized merge.`);
    continue;
  }

  if (isMerged(out)) {
    // Self-merge path (serializedMerge=false, or an older ship-card).
    mergedTitles.add(title);
    allResults.push({ wave: card._wave || 0, ...out });
    log(`wave-ship: MERGED "${title}" (${mergedTitles.size} merged so far).`);
    continue;
  }

  // Only BLOCKED (no-PR) cards are safe to auto-retry — see isRetriable.
  if (
    isRetriable(out) &&
    (remCount.get(title) || 0) < MAX_REMEDIATION &&
    !budgetLow()
  ) {
    const cil = out.result?.cil || null;
    const question = out.result?.detail?.question || null;
    const options = out.result?.detail?.questionOptions || [];

    // Tier-2 E clarify gate: a card blocked on a DECISION asks one question.
    // Resolve it (supplied answer → resolver agent); if unresolved, escalate to a
    // human via decisionGates and DO NOT retry (no guessing past a real decision).
    if (question) {
      phase("Reconcile");
      const resolved = await resolveQuestion(
        { ...card, cil },
        question,
        options,
      );
      if (!resolved) {
        decisionGates.push({ title, cil, question, options });
        failedTitles.add(title);
        allResults.push({
          wave: card._wave || 0,
          card,
          result: {
            status: "needs-decision",
            cil,
            pr: null,
            prUrl: null,
            note: question,
          },
          error: question,
        });
        log(`wave-ship: "${title}" needs a HUMAN decision — ${question}`);
        continue;
      }
      remCount.set(title, (remCount.get(title) || 0) + 1);
      const retry = normCard({
        title,
        task: card.task,
        plan: card.plan,
        scope: `${card.scope}\n\nRESOLVED DECISION (${resolved.source}) — ${question}\n→ ${resolved.answer}`,
        labels: card.labels,
        priority: card.priority,
        dependsOn: [],
        cil,
        migration: card.migration,
      });
      retry._wave = card._wave;
      byTitle.set(title, retry);
      pool.push(retry);
      log(
        `wave-ship: resolved decision for "${title}" (${resolved.source}) — requeued.`,
      );
      continue;
    }

    // No decision question → scope-guess remediation (reconcile refines by cil).
    remCount.set(title, (remCount.get(title) || 0) + 1);
    phase("Reconcile");
    const fix = await reconcile("remediate", {
      waveLabel: `card "${title}"`,
      failures: [toFailure(out)],
    });
    if (Array.isArray(fix?.blockers)) reconcileBlockers.push(...fix.blockers);
    // Refinement is matched back by cil (stable), never by a hallucinated title;
    // the retry card is rebuilt 1:1 from the original so its identity can't drift.
    const refine =
      (fix?.newWaveCards || []).find((fc) => fc && fc.cil && fc.cil === cil) ||
      {};
    const retry = normCard({
      title,
      task: refine.task || card.task,
      plan: refine.plan || card.plan,
      scope: refine.scope || card.scope,
      labels: card.labels,
      priority: card.priority,
      dependsOn: [],
      cil, // resolve the existing ticket instead of creating a duplicate PR
      migration: card.migration,
    });
    retry._wave = card._wave;
    byTitle.set(title, retry);
    pool.push(retry);
    log(
      `wave-ship: remediation ${remCount.get(title)} — requeued blocked card "${title}".`,
    );
    continue;
  }

  // Terminal non-merge (needs-attention / merge-failed / error / gave-up). Its
  // title enters failedTitles so dependents cascade-fail in step 1.
  failedTitles.add(title);
  allResults.push({ wave: card._wave || 0, ...out });
  log(`wave-ship: card "${title}" terminal — ${statusOf(out)}.`);
}

const wavesDeployed = plannedWaves.length + continuations;

// ── Phase 4: report ──────────────────────────────────────────────────────────
phase("Report");
const cards = allResults.map((r) => ({
  wave: r.wave,
  title: r.card.title,
  status: statusOf(r),
  cil: r.result?.cil || null,
  pr: r.result?.pr || null,
  prUrl: r.result?.prUrl || null,
  mergeSha: r.result?.mergeSha || null,
  ticketDone: !!r.result?.ticketDone,
  error: r.error || null,
}));
const mergedCards = cards.filter((c) => c.status === "merged");
const failedCards = cards.filter((c) => c.status !== "merged");

let narrative = null;
try {
  narrative = await agent(
    `Write a concise markdown status report for a wave-ship run on ${REPO}.
OBJECTIVE: ${GOAL || PLAN}
WAVES DEPLOYED: ${wavesDeployed}${stopped ? ` (stopped: ${stopped})` : ""}
CARDS (JSON): ${JSON.stringify(cards, null, 2)}
Lead with the outcome (merged X/Y), list merged PRs by title + url, then anything needing attention with a one-line next action. No fluff.`,
    maybeModel({
      label: "report",
      phase: "Report",
      agentType: "general-purpose",
      effort: "low",
    }),
  );
} catch (_e) {
  narrative = null;
}

log(
  `wave-ship: DONE — ${mergedCards.length}/${cards.length} cards merged across ${wavesDeployed} wave(s)${stopped ? ` (stopped: ${stopped})` : ""}.`,
);

return {
  status: stopped
    ? `stopped:${stopped}`
    : failedCards.length
      ? "complete-with-failures"
      : "complete",
  objective: GOAL || PLAN,
  wavesDeployed,
  merged: mergedCards.length,
  failed: failedCards.length,
  cards,
  mergedPrUrls: mergedCards.map((c) => c.prUrl).filter(Boolean),
  blockers: [
    ...failedCards.map(
      (c) => `${c.title} [${c.status}]${c.error ? `: ${c.error}` : ""}`,
    ),
    ...decisionGates.map((g) => `${g.title} NEEDS DECISION: ${g.question}`),
    ...reconcileBlockers,
  ].filter((v, i, arr) => v && arr.indexOf(v) === i),
  decisionGates,
  narrative,
};
