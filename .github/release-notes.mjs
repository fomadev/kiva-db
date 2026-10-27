// Writes the text of a GitHub release from CHANGELOG.md and the history:
// what the version brings (its section of the changelog), the pull
// requests merged since the previous version, and who contributed.
//
//   node .github/release-notes.mjs v2.0.0 > notes.md
//
// Needs the git history with its tags (actions/checkout with fetch-depth: 0)
// and the GitHub CLI, logged in (GH_TOKEN in a workflow).
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const tag = process.argv[2];
if (!/^v\d+\.\d+\.\d+$/.test(tag ?? "")) {
  console.error("usage: node .github/release-notes.mjs v<major>.<minor>.<patch>");
  process.exit(2);
}
const version = tag.slice(1);
const repo = process.env.GITHUB_REPOSITORY || "fomadev/kiva-db";
const run = (cmd, args) => execFileSync(cmd, args, { encoding: "utf8" }).trim();
const gh = (path) => JSON.parse(run("gh", ["api", path]));

// --- The section of the changelog --------------------------------------------
const changelog = readFileSync(new URL("../CHANGELOG.md", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const heading = new RegExp(`^## ${version.replace(/\./g, "\\.")} - (\\d{4}-\\d{2}-\\d{2})\\s*$`, "m");
const found = changelog.match(heading);
if (!found) {
  console.error(
    `CHANGELOG.md has no section "## ${version} - <date>": write what ${version} brings there ` +
      `(rename "## Unreleased (${version})" once the version is ready), then tag again.`
  );
  process.exit(1);
}
const date = found[1];
const start = found.index + found[0].length;
const next = changelog.slice(start).search(/^## /m);
const section = changelog.slice(start, next < 0 ? undefined : start + next).trim();
// Links to files of the repository work from the release page too.
const absolute = (text) =>
  text.replace(/\]\((?!https?:|#)([^)\s]+)\)/g, (_, path) => `](https://github.com/${repo}/blob/${tag}/${path})`);
const [summary, ...parts] = section.split(/^### /m);
const subsections = parts.map((p) => {
  const [title, ...body] = p.split("\n");
  return `## ${title.trim()}\n\n${absolute(body.join("\n").trim())}`;
});

// --- Pull requests and contributors since the previous version --------------
const tags = run("git", ["tag", "--list", "v*", "--sort=-v:refname"]).split("\n").filter(Boolean);
const previous = tags.find((t) => t !== tag && compare(t.slice(1), version) < 0);
const range = previous ? `${previous}...${tag}` : null;
const isBot = (login) => !login || /\[bot\]$|-bot$/.test(login);

const commits = range ? gh(`repos/${repo}/compare/${range}`).commits : [];
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
  const merged = subject.match(/^Merge pull request #(\d+)/);
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
if (summary.trim()) out.push(absolute(summary.trim()), "");
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
  "* Already installed: `kivadb update`, then restart KivaDB.",
  `* Linux and macOS: \`curl -sSL https://raw.githubusercontent.com/${repo}/main/install.sh | bash\``,
  `* Windows (PowerShell): \`irm https://raw.githubusercontent.com/${repo}/main/install.ps1 | iex\``,
  `* Docker: \`docker pull fomadev0/kiva-db:${version}\``,
  "* Or download the archive for your system below, and check it against `SHA256SUMS.txt`.",
  "",
  `The full manual: [DOCUMENTATION.md](https://github.com/${repo}/blob/${tag}/DOCUMENTATION.md).` +
    (range ? ` Every change: [${previous}...${tag}](https://github.com/${repo}/compare/${range}).` : "")
);
console.log(out.join("\n"));

function compare(a, b) {
  const x = a.split(".").map(Number);
  const y = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
  return 0;
}
