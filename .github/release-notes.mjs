// Writes the text of a GitHub release from CHANGELOG.md and the history:
// what the version brings (its section of the changelog), the pull
// requests merged since the previous version, and who contributed.
//
//   node .github/release-notes.mjs v2.0.0 > notes.md
//   node .github/release-notes.mjs v2.0.0-rc.1 > notes.md
//
// A pre-release (`v2.0.0-rc.1`, `v2.1.0-alpha.1`, `v2.1.0-beta.2`) takes the
// section "Unreleased (2.0.0)", which grows until the release itself gets its
// dated section. Its notes keep only what that section gained since the
// previous tag, release or pre-release: the entries added or rewritten, and
// the pull requests merged in between. The notes of a release cover
// everything since the previous release.
//
// Needs the git history with its tags (actions/checkout with fetch-depth: 0)
// and the GitHub CLI, logged in (GH_TOKEN in a workflow).
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const tag = process.argv[2];
const parsed = (tag ?? "").match(/^v(\d+\.\d+\.\d+)(?:-([0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*))?$/);
if (!parsed) {
  console.error("usage: node .github/release-notes.mjs v<major>.<minor>.<patch>[-rc.<n>|-alpha.<n>|-beta.<n>]");
  process.exit(2);
}
const version = tag.slice(1);
// The version it is a pre-release of (or itself), and the pre-release.
const [, core, candidate] = parsed;
const kind =
  { rc: "release candidate", alpha: "alpha version", beta: "beta version" }[candidate?.split(".")[0]] ?? "pre-release";
const repo = process.env.GITHUB_REPOSITORY || "fomadev/kiva-db";
// The answers of GitHub can be large: a comparison since 1.7.1 lists every
// changed file, past the 1 MB that execFileSync keeps by default.
const run = (cmd, args) => execFileSync(cmd, args, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 }).trim();
const gh = (path) => JSON.parse(run("gh", ["api", path]));

// --- The section of the changelog --------------------------------------------
// The tag must name the version the executables were built as.
const cargo = readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8").match(/^version = "([^"]+)"/m)?.[1];
if (cargo !== version) {
  console.error(`Cargo.toml says version ${cargo}, the tag ${tag}: set version = "${version}" there, then tag again.`);
  process.exit(1);
}

const changelog = readFileSync(new URL("../CHANGELOG.md", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const escaped = core.replace(/\./g, "\\.");
const heading = candidate
  ? new RegExp(`^## Unreleased \\(${escaped}\\)\\s*$`, "m")
  : new RegExp(`^## ${escaped} - (\\d{4}-\\d{2}-\\d{2})\\s*$`, "m");
const found = changelog.match(heading);
if (!found) {
  console.error(
    candidate
      ? `CHANGELOG.md has no section "## Unreleased (${core})" for the candidate ${version}.`
      : `CHANGELOG.md has no section "## ${version} - <date>": write what ${version} brings there ` +
          `(rename "## Unreleased (${version})" once the version is ready), then tag again.`
  );
  process.exit(1);
}
const date = found[1] ?? new Date().toISOString().slice(0, 10);
const section = sectionAt(changelog, found);
// Links to files of the repository work from the release page too.
const absolute = (text) =>
  text.replace(/\]\((?!https?:|#)([^)\s]+)\)/g, (_, path) => `](https://github.com/${repo}/blob/${tag}/${path})`);

// --- The previous version ------------------------------------------------------
const older = run("git", ["tag", "--list", "v*"])
  .split("\n")
  .filter((t) => /^v\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(t) && compare(t.slice(1), version) < 0)
  .sort((a, b) => compare(b.slice(1), a.slice(1)));
// A release: the previous release (2.0.0 covers everything since 1.7.1). A
// pre-release: the tag just before it, whatever its kind (rc.8 for rc.9).
const previous = candidate ? older[0] : older.find((t) => !t.includes("-"));
const range = previous ? `${previous}...${tag}` : null;

// --- What the section says ------------------------------------------------------
// For a pre-release, the entries the same section held at the previous tag
// are left out: what is left was added or rewritten since.
let before = [];
if (candidate && previous) {
  let old = "";
  try {
    old = run("git", ["show", `${previous}:CHANGELOG.md`]).replace(/\r\n/g, "\n");
  } catch {
    // No changelog at that tag: everything is new.
  }
  const was = old.match(heading);
  if (was) before = entries(sectionAt(old, was));
}
// An entry rewritten since (it starts as one of before) keeps only the
// sentences it gained, after its lead ("Collections:") if it has one.
const kept = entries(section).flatMap((e) => {
  if (before.some((b) => b.text === e.text)) return [];
  const was = before.find((b) => b.part === e.part && b.text.slice(0, 60) === e.text.slice(0, 60));
  if (!was) return [e];
  const known = new Set(sentences(was.text));
  const added = sentences(e.text).filter((s) => !known.has(s));
  if (!added.length) return [];
  const lead = e.text.match(/^([*-] )([^.:]{1,40}:)\s/);
  let first = added[0].startsWith(e.text.slice(0, 2)) ? added[0].slice(2) : added[0];
  let head = "";
  if (lead && !first.startsWith(lead[2])) {
    head = `${lead[2]} `;
    // "Collections: the note ...", not "Collections: The note ...".
    if (/^[A-Z][a-z]/.test(first)) first = first[0].toLowerCase() + first.slice(1);
  }
  return [{ part: e.part, text: `${e.text.slice(0, 2)}${head}${[first, ...added.slice(1)].join(" ")}` }];
});
const summary = kept
  .filter((e) => !e.part)
  .map((e) => e.text)
  .join("\n\n");
const subsections = [...new Set(kept.filter((e) => e.part).map((e) => e.part))].map((part) => {
  const body = kept
    .filter((e) => e.part === part)
    .map((e) => e.text)
    .join("\n");
  return `## ${part}\n\n${absolute(body)}`;
});

// --- Pull requests and contributors since the previous version --------------
const isBot = (login) => !login || /\[bot\]$|-bot$/.test(login);

// Only the fields used below, page by page: the comparison also holds every
// changed file, and more than 250 commits only come with pages.
const commitFields =
  ".commits[] | {parents: [.parents[].sha], author: (if .author then {login: .author.login} else null end), " +
  "commit: {message: .commit.message, author: {name: .commit.author.name}}}";
const commits = range
  ? run("gh", ["api", "--paginate", `repos/${repo}/compare/${range}?per_page=100`, "--jq", commitFields])
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line))
  : [];
const people = new Map();
const credit = (login, name, what) => {
  const who = login ? `@${login}` : name;
  const p = people.get(who) || { commits: 0, prs: [] };
  if (what === "commit") p.commits++;
  else p.prs.push(what);
  people.set(who, p);
};
const prNumbers = [];
for (const c of commits) {
  const subject = c.commit.message.split("\n")[0];
  // A merge commit, or a squashed pull request ("Title (#106)").
  const merged = subject.match(/^Merge pull request #(\d+)/) ?? (c.parents.length === 1 && subject.match(/\(#(\d+)\)$/));
  if (merged) {
    prNumbers.push(Number(merged[1]));
    continue;
  }
  // Merges of the main branch into a branch are not work of their own.
  if (c.parents.length > 1 || isBot(c.author?.login ?? (c.commit.author.name || null))) continue;
  credit(c.author?.login, c.commit.author.name, "commit");
}
const prs = prNumbers.sort((a, b) => a - b).map((n) => gh(`repos/${repo}/pulls/${n}`));
for (const pr of prs) {
  if (!isBot(pr.user.login)) credit(pr.user.login, null, `#${pr.number}`);
}

// --- The text -----------------------------------------------------------------
const out = [];
out.push(`KivaDB **${version}**, released on ${date}.`, "");
if (candidate) {
  out.push(
    absolute(
      `${/^[aeiou]/.test(kind) ? "An" : "A"} ${kind} of ${core}, to try before ${core} itself is released. ` +
        "`kivadb update` does not install it, nor the install scripts unless asked (below). Read " +
        "[the upgrade guide](DOCUMENTATION.md#chapter-33--upgrading-from-kivadb-1x-to-20) first."
    ),
    ""
  );
  if (previous) {
    out.push(
      summary || subsections.length
        ? `What changed since ${previous.slice(1)}. Everything in ${core} so far: ` +
            `[CHANGELOG.md](https://github.com/${repo}/blob/${tag}/CHANGELOG.md).`
        : `The changelog records no change since ${previous.slice(1)}: the pull requests below are what changed.`,
      ""
    );
  }
}
if (summary) out.push(absolute(summary), "");
out.push(...subsections.flatMap((s) => [s, ""]));
if (prs.length) {
  out.push("## Pull requests", "");
  for (const pr of prs) out.push(`* ${pr.title} (#${pr.number}) by @${pr.user.login}`);
  out.push("");
}
if (people.size) {
  out.push("## Contributors", "");
  out.push(
    `Thanks to everyone who worked on ${version}:`,
    "",
    ...[...people].map(([who, p]) => {
      const what = [];
      if (p.commits) what.push(`${p.commits} commit${p.commits > 1 ? "s" : ""}`);
      if (p.prs.length) what.push(`pull request${p.prs.length > 1 ? "s" : ""} ${p.prs.join(", ")}`);
      return `* ${who}: ${what.join(", ")}`;
    }),
    ""
  );
}
out.push(
  "## Install or update",
  "",
  ...(candidate ? [] : ["* Already installed: `kivadb update`, then restart KivaDB."]),
  // The install scripts take the latest release unless told which version.
  `* Linux and macOS: \`curl -sSL https://raw.githubusercontent.com/${repo}/main/install.sh | ` +
    (candidate ? `KIVADB_VERSION=${version} bash\`` : "bash`"),
  `* Windows (PowerShell): \`` +
    (candidate ? `$env:KIVADB_VERSION="${version}"; ` : "") +
    `irm https://raw.githubusercontent.com/${repo}/main/install.ps1 | iex\``,
  `* Docker: \`docker pull fomadev0/kiva-db:${version}\``,
  "* Or download the archive for your system below, and check it against `SHA256SUMS.txt`.",
  "",
  `The full manual: [DOCUMENTATION.md](https://github.com/${repo}/blob/${tag}/DOCUMENTATION.md).` +
    (range ? ` Every change: [${previous}...${tag}](https://github.com/${repo}/compare/${range}).` : "")
);
console.log(out.join("\n"));

// The text of a section of the changelog, from its heading to the next one.
function sectionAt(text, heading) {
  const start = heading.index + heading[0].length;
  const next = text.slice(start).search(/^## /m);
  return text.slice(start, next < 0 ? undefined : start + next).trim();
}

// The entries of a section: each bullet with the subsection it is in, and
// the paragraphs before the first subsection (part ""). A bullet runs to the
// next bullet or subsection.
function entries(text) {
  const out = [];
  let part = "";
  let cur = null;
  const end = () => {
    if (cur) out.push({ part: cur.part, text: cur.lines.join("\n").trim() });
    cur = null;
  };
  for (const line of text.split("\n")) {
    if (line.startsWith("### ")) {
      end();
      part = line.slice(4).trim();
    } else if (line.startsWith("* ") || line.startsWith("- ")) {
      end();
      cur = { part, lines: [line] };
    } else if (!line.trim() && !part) {
      end();
    } else if (cur) {
      cur.lines.push(line);
    } else if (line.trim()) {
      cur = { part, lines: [line] };
    }
  }
  end();
  return out.filter((e) => e.text);
}

// The sentences of an entry: a full stop, then a space and a capital, a
// backquote or a link. Links and code may hold full stops of their own.
function sentences(text) {
  return text
    .replace(/\s*\n\s*/g, " ")
    .split(/(?<=\.\)?)\s+(?=[A-Z`[(*])/)
    .map((s) => s.trim())
    .filter(Boolean);
}

// The order of versions: 2.0.0-alpha.1 < 2.0.0-beta.1 < 2.0.0-rc.2 <
// 2.0.0-rc.10 < 2.0.0 < 2.0.1.
function compare(a, b) {
  const [ca, pa] = a.split(/-(.*)/s);
  const [cb, pb] = b.split(/-(.*)/s);
  const x = ca.split(".").map(Number);
  const y = cb.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
  if (!pa || !pb) return (pa ? -1 : 0) - (pb ? -1 : 0);
  const u = pa.split(".");
  const v = pb.split(".");
  for (let i = 0; i < Math.max(u.length, v.length); i++) {
    if (u[i] === undefined) return -1;
    if (v[i] === undefined) return 1;
    const nu = /^\d+$/.test(u[i]);
    const nv = /^\d+$/.test(v[i]);
    if (nu && nv && Number(u[i]) !== Number(v[i])) return Number(u[i]) - Number(v[i]);
    if (nu !== nv) return nu ? -1 : 1;
    if (!nu && !nv && u[i] !== v[i]) return u[i] < v[i] ? -1 : 1;
  }
  return 0;
}
