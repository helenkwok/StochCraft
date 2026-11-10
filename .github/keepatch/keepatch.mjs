// Keepatch: a slow, resumable maintenance report for a public repository.
// Reads pull requests, checks and bug issues; ranks them with fixed rules; keeps one report issue
// and one issue per task that needs work. It never touches code, branches or merges.
//
//   GITHUB_TOKEN=... GITHUB_REPOSITORY=owner/name node keepatch.mjs
//   KEEPATCH_DRY_RUN=1          print what it would write, write nothing
//   KEEPATCH_TINYDECIDE_DIR=dir add TinyDecide scores as a shadow column (never used for ranking)
//   node keepatch.mjs --self-test
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const LABEL = "keepatch";
const STALE_DAYS = 30;
const BOTS = new Set(["dependabot[bot]", "renovate[bot]"]);
const SECURITY = /GHSA-[0-9a-z]{4}-|CVE-\d{4}-\d+|\bsecurity\b|\bvulnerab/i;

// ---- pure rules ---------------------------------------------------------------------------

// Check runs and commit statuses at one commit -> "failing" | "running" | "passing" | "none".
// "none" also covers checks that only skipped: a green tick with nothing behind it is not a pass.
export function checkState(runs, statuses = []) {
  const bad = ["failure", "timed_out", "cancelled", "action_required", "startup_failure"];
  if (runs.some((r) => bad.includes(r.conclusion)) || statuses.some((s) => s.state === "failure" || s.state === "error")) return "failing";
  if (runs.some((r) => r.status !== "completed") || statuses.some((s) => s.state === "pending")) return "running";
  if (runs.some((r) => r.conclusion === "success") || statuses.some((s) => s.state === "success")) return "passing";
  return "none";
}

// One task -> { priority, reasons }. Higher first. Every point is explained in `reasons`.
export function rank(t) {
  const reasons = [];
  let p = 0;
  const add = (n, why) => { p += n; reasons.push(why); };
  if (t.kind === "main") add(100, "checks fail on the default branch");
  if (t.kind === "bug") add(40, "open bug report");
  if (t.kind === "pr") {
    if (t.security) add(90, "mentions a security advisory");
    if (t.checks === "failing") add(t.bot ? 70 : 60, "checks fail");
    if (t.checks === "none") add(55, "no check ran on the current commit");
    if (t.checks === "passing" && !t.draft) add(t.bot ? 50 : 45, "passes checks and waits for a merge");
    if (t.checks === "passing" && !t.reviewedAtHead) reasons.push("no review of the current commit");
    if (t.conflict) add(20, "conflicts with the base branch");
    if (t.ageDays >= STALE_DAYS) add(30, `open for ${t.ageDays} days`);
  }
  if (t.kind !== "main") p += Math.min(t.ageDays, 20) / 2; // older first within a tier, capped
  return { priority: Math.round(p), reasons };
}

// Was the commit `head` reviewed? Either a pull request review of that commit, or a review bot's
// summary comment that names it. CodeRabbit posts no pull request review when it has nothing to
// flag: it edits one comment that lists the commit range it read (seen on StochCraft #97).
// ponytail: text match on the bot's comment; replace if the bot offers a structured record.
const REVIEW_BOTS = new Set(["coderabbitai[bot]"]);
const NOT_A_REVIEW = /review in progress|currently processing|review skipped|rate limit/i;
export const reviewedAt = (head, reviews, comments = []) =>
  reviews.some((r) => r.commit_id === head && r.state !== "PENDING") ||
  comments.some((c) => REVIEW_BOTS.has(c.user?.login) && c.body.includes(head) && !NOT_A_REVIEW.test(c.body));

// A task gets its own issue only when something has to be done about it.
export const needsIssue = (t) => t.kind === "main" || (t.kind === "pr" && (t.checks === "failing" || t.checks === "none"));

// Titles and other outside text are untrusted: show them as inline code so they cannot mention
// people, add links or break the table.
export const safe = (s) => "`" + String(s).replace(/[`|\r\n]/g, " ").slice(0, 110) + "`";

const marker = (key) => `<!-- keepatch:key=${key} -->`;
export function readState(body) {
  const m = /<!-- keepatch:state (\{.*?\}) -->/s.exec(body || "");
  try { return m ? JSON.parse(m[1]) : null; } catch { return null; }
}
export const keyOf = (body) => /<!-- keepatch:key=([\w-]+) -->/.exec(body || "")?.[1] ?? null;

function selfTest() {
  assert.equal(checkState([]), "none");
  assert.equal(checkState([{ status: "completed", conclusion: "skipped" }]), "none");
  assert.equal(checkState([{ status: "completed", conclusion: "success" }, { status: "in_progress" }]), "running");
  assert.equal(checkState([{ status: "completed", conclusion: "success" }], [{ state: "failure" }]), "failing");
  assert.equal(checkState([{ status: "completed", conclusion: "success" }, { status: "completed", conclusion: "skipped" }]), "passing");
  const pr = (o) => rank({ kind: "pr", ageDays: 1, ...o }).priority;
  assert.ok(rank({ kind: "main" }).priority > pr({ security: true, checks: "passing", bot: true }) - 60);
  assert.ok(pr({ security: true, checks: "passing", bot: true }) > pr({ checks: "failing", bot: true }));
  assert.ok(pr({ checks: "failing", bot: true }) > pr({ checks: "none" }));
  assert.ok(pr({ checks: "none" }) > pr({ checks: "passing", bot: true }));
  assert.ok(pr({ checks: "passing", bot: true }) > rank({ kind: "bug", ageDays: 1 }).priority);
  assert.equal(needsIssue({ kind: "pr", checks: "passing" }), false);
  const bot = (body) => [{ user: { login: "coderabbitai[bot]" }, body }];
  assert.equal(reviewedAt("abc", [{ commit_id: "abc", state: "COMMENTED" }]), true);
  assert.equal(reviewedAt("abc", [{ commit_id: "old", state: "APPROVED" }]), false);
  assert.equal(reviewedAt("abc", [], bot("between base and abc. No actionable comments")), true);
  assert.equal(reviewedAt("abc", [], bot("between base and abc. Review in progress")), false);
  assert.equal(reviewedAt("abc", [], bot("Rate limit exceeded for abc")), false);
  assert.equal(reviewedAt("abc", [], [{ user: { login: "someone" }, body: "abc looks fine" }]), false);
  assert.equal(safe("hi @someone `x` | y\n[a](b)"), "`hi @someone  x    y [a](b)`");
  const body = `${marker("pr-12")}\n<!-- keepatch:state {"runs":3,"head":"abc"} -->`;
  assert.equal(keyOf(body), "pr-12");
  assert.deepEqual(readState(body), { runs: 3, head: "abc" });
  assert.equal(readState("<!-- keepatch:state {oops} -->"), null);
  console.log("self-test ok");
}

// ---- GitHub -------------------------------------------------------------------------------

const repo = process.env.GITHUB_REPOSITORY;
const DRY = !!process.env.KEEPATCH_DRY_RUN;
async function gh(method, path, body) {
  const res = await fetch(`https://api.github.com/repos/${repo}${path}`, {
    method,
    headers: { authorization: `Bearer ${process.env.GITHUB_TOKEN}`, accept: "application/vnd.github+json", "x-github-api-version": "2022-11-28", "user-agent": "keepatch" },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${(await res.text()).slice(0, 200)}`);
  return res.status === 204 ? null : res.json();
}
// ponytail: first 100 of each list, no paging. Add paging when a repo has more open items.
const list = (path) => gh("GET", path + (path.includes("?") ? "&" : "?") + "per_page=100");
async function write(what, method, path, body) {
  console.log(`${DRY ? "would" : "did"}: ${what}`);
  return DRY ? null : gh(method, path, body);
}
const days = (iso, now) => Math.floor((now - Date.parse(iso)) / 864e5);
async function checksAt(sha) {
  const [runs, st] = await Promise.all([list(`/commits/${sha}/check-runs`), gh("GET", `/commits/${sha}/status`)]);
  return { state: checkState(runs.check_runs, st.statuses), failing: runs.check_runs.filter((r) => r.conclusion === "failure").map((r) => r.name) };
}

async function collect(now) {
  const info = await gh("GET", "");
  const tasks = [];
  const main = await checksAt(info.default_branch);
  if (main.state === "failing") tasks.push({ kind: "main", key: "main-ci", title: `Checks fail on ${info.default_branch}`, url: `${info.html_url}/actions`, checks: "failing", failing: main.failing, ageDays: 0 });
  for (const pr of await list("/pulls?state=open")) {
    const [c, reviews, comments] = await Promise.all([checksAt(pr.head.sha), list(`/pulls/${pr.number}/reviews`), list(`/issues/${pr.number}/comments`)]);
    const full = await gh("GET", `/pulls/${pr.number}`); // `mergeable` is only on the single-PR endpoint
    tasks.push({
      kind: "pr", key: `pr-${pr.number}`, title: pr.title, url: pr.html_url, head: pr.head.sha, checks: c.state, failing: c.failing,
      bot: BOTS.has(pr.user.login), draft: pr.draft, conflict: full.mergeable === false,
      security: SECURITY.test(pr.title) || SECURITY.test(pr.body || "") || pr.labels.some((l) => /security/i.test(l.name)),
      reviewedAtHead: reviewedAt(pr.head.sha, reviews, comments),
      ageDays: days(pr.created_at, now),
    });
  }
  for (const i of await list("/issues?state=open&labels=bug")) {
    if (!i.pull_request && !i.labels.some((l) => l.name === LABEL)) tasks.push({ kind: "bug", key: `bug-${i.number}`, title: i.title, url: i.html_url, ageDays: days(i.created_at, now) });
  }
  return { info, mainState: main.state, tasks: tasks.map((t) => ({ ...t, ...rank(t) })).sort((a, b) => b.priority - a.priority) };
}

// ---- TinyDecide, shadow only --------------------------------------------------------------

async function shadow(tasks) {
  const dir = process.env.KEEPATCH_TINYDECIDE_DIR;
  if (!dir) return null;
  try {
    const { TinyDecide } = await import(pathToFileURL(join(dir, "tinydecide.js")));
    const bin = readFileSync(join(dir, "model.bin"));
    const model = new TinyDecide(JSON.parse(readFileSync(join(dir, "meta.json"), "utf8")), bin.buffer.slice(bin.byteOffset, bin.byteOffset + bin.byteLength));
    const qs = [{ type: "noul", text: "This describes something that is broken." }, { type: "noul", text: "This change fixes a security vulnerability." }];
    return new Map(tasks.map((t) => [t.key, model.answer({ state: t.title, questions: qs }).answers.map((a) => a.p.toFixed(2)).join(" / ")]));
  } catch (e) {
    console.log(`TinyDecide not loaded (${e.message}); report continues without it`);
    return null;
  }
}

// ---- report and task issues ---------------------------------------------------------------

function reportBody({ info, mainState, tasks }, td, now, stateJson) {
  const rows = tasks.map((t) => `| ${t.priority} | [${t.key}](${t.url}) | ${safe(t.title)} | ${t.checks ?? ""} | ${t.reasons.join("; ")} |${td ? ` ${td.get(t.key) ?? ""} |` : ""}`);
  return [
    marker("report"), `<!-- keepatch:state ${stateJson} -->`,
    `Updated ${new Date(now).toISOString().slice(0, 16)}Z by the scheduled Keepatch run. This issue is rewritten on every run.`,
    "", `**${info.default_branch}:** checks ${mainState}. **Queue:** ${tasks.length} item${tasks.length === 1 ? "" : "s"}.`, "",
    tasks.length ? `| Priority | Item | Title | Checks | Why |${td ? " TinyDecide: broken / security (shadow, not used) |" : ""}\n|---|---|---|---|---|${td ? "---|" : ""}\n${rows.join("\n")}` : "Nothing is waiting.",
    "", "Priority comes from fixed rules in `.github/keepatch/keepatch.mjs`. A passing check here means at least one check ran and succeeded on the current commit. Keepatch reports; people merge.",
  ].join("\n");
}

function taskBody(t, st) {
  return [
    marker(t.key), `<!-- keepatch:state ${JSON.stringify(st)} -->`,
    `${t.url}`, "", `**Why this is open:** ${t.reasons.join("; ")}.`,
    t.failing?.length ? `**Failing checks:** ${t.failing.map(safe).join(", ")}` : "",
    t.head ? `**Commit examined:** \`${t.head.slice(0, 12)}\`` : "", `**Seen on ${st.runs} run${st.runs === 1 ? "" : "s"}, first on ${st.first}.**`,
    "", "Keepatch closes this issue when the problem is gone. Link a fix with `Closes #<this issue>`.",
  ].filter((l) => l !== "").join("\n\n");
}

async function main() {
  if (!repo || !process.env.GITHUB_TOKEN) throw new Error("set GITHUB_REPOSITORY and GITHUB_TOKEN");
  const now = Date.now(), today = new Date(now).toISOString().slice(0, 10);
  const data = await collect(now);
  const td = await shadow(data.tasks);
  const mine = (await list(`/issues?state=open&labels=${LABEL}`)).filter((i) => !i.pull_request && keyOf(i.body));
  const byKey = new Map(mine.map((i) => [keyOf(i.body), i]));
  if (!DRY) await gh("POST", "/labels", { name: LABEL, color: "5319e7", description: "Opened and updated by the Keepatch maintenance run" }).catch(() => {}); // exists already: fine

  const active = new Set();
  for (const t of data.tasks.filter(needsIssue)) {
    active.add(t.key);
    const old = byKey.get(t.key), prev = readState(old?.body);
    const st = { head: t.head ?? null, checks: t.checks, first: prev?.first ?? today, runs: (prev?.runs ?? 0) + 1 };
    const title = `Keepatch: ${t.kind === "main" ? t.title : `#${t.key.slice(3)} ${t.checks === "none" ? "has no check on its current commit" : "fails checks"}`}`;
    if (old) await write(`update issue #${old.number} (${t.key}, run ${st.runs})`, "PATCH", `/issues/${old.number}`, { title, body: taskBody(t, st) });
    else await write(`open issue for ${t.key}`, "POST", "/issues", { title, body: taskBody(t, st), labels: [LABEL] });
  }
  for (const [key, issue] of byKey) {
    if (key === "report" || active.has(key)) continue;
    await write(`close issue #${issue.number} (${key} no longer needs work)`, "POST", `/issues/${issue.number}/comments`, { body: "The problem is gone on the latest run. Closing." });
    if (!DRY) await gh("PATCH", `/issues/${issue.number}`, { state: "closed", state_reason: "completed" });
  }

  const prevRuns = readState(byKey.get("report")?.body)?.runs ?? 0;
  const body = reportBody(data, td, now, JSON.stringify({ runs: prevRuns + 1, last: today }));
  const report = byKey.get("report");
  if (report) await write(`rewrite report issue #${report.number}`, "PATCH", `/issues/${report.number}`, { body });
  else await write("open the report issue", "POST", "/issues", { title: "Keepatch maintenance report", body, labels: [LABEL] });
  console.log("\n" + body);
}

if (process.argv.includes("--self-test")) selfTest();
else await main();
