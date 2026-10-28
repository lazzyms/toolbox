// Builds a release body from the "## Release notes" sections of the PRs merged since the previous tag.
// Usage: node scripts/release-notes.mjs <version> <previous-tag> <source-sha> > release-notes.md
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = 'lazzyms/toolbox';
export const groups = ['New', 'Fixes', 'Website', 'Behind the scenes', 'Other changes'];
const other = 'Other changes';
const header = resolve(dirname(fileURLToPath(import.meta.url)), '../../.github/release-notes-header.md');

const groupFor = (heading) => groups.find((group) => group.toLowerCase() === heading.trim().toLowerCase()) ?? other;

// Returns [{group, text}] from a PR body, null when it has no filled-in section,
// or [] when the author marked it "None" (nothing worth telling users).
export const parseReleaseNotes = (body) => {
  const lines = (body ?? '').replace(/<!--[\s\S]*?-->/g, '').split(/\r?\n/);
  const start = lines.findIndex((line) => /^##\s+release notes\s*$/i.test(line.trim()));
  if (start === -1) return null;
  const entries = [];
  let group = other;
  for (const line of lines.slice(start + 1)) {
    if (/^##\s/.test(line)) break;
    const heading = line.match(/^###\s+(.+)/);
    const bullet = line.match(/^\s{0,1}[-*]\s+(.+)/);
    if (heading) group = groupFor(heading[1]);
    else if (bullet) entries.push({ group, text: bullet[1].trim() });
    else if (line.trim() && entries.length) entries.at(-1).text += ` ${line.trim()}`;
  }
  if (entries.length === 1 && /^none\.?$/i.test(entries[0].text)) return [];
  return entries.length ? entries : null;
};

const prLink = (number) => `[#${number}](https://github.com/${repo}/pull/${number})`;

// commits: [{subject, pr?: {number, title, body}}], oldest or newest first.
export const renderReleaseNotes = ({ version, previousTag, commits }) => {
  const entries = [];
  for (const { subject, pr } of commits) {
    if (/\[skip ci\]/i.test(subject)) continue;
    const parsed = pr ? parseReleaseNotes(pr.body) : null;
    if (parsed) entries.push(...parsed.map((entry) => ({ ...entry, number: pr.number })));
    else entries.push({ group: other, text: pr ? pr.title : subject, number: pr?.number });
  }
  const sections = groups
    .map((group) => [group, entries.filter((entry) => entry.group === group)])
    .filter(([, items]) => items.length)
    .map(([group, items]) => [`### ${group}`, ...items.map(({ text, number }) =>
      `- ${text}${number && !text.includes(`/pull/${number}`) ? ` (${prLink(number)})` : ''}`)].join('\n'));
  return [
    `## What's changed in ${version}`,
    ...(sections.length ? sections : [`No changes since ${previousTag}.`]),
    `**Full Changelog**: https://github.com/${repo}/compare/${previousTag}...tauri-v${version}`,
  ].join('\n\n') + '\n';
};

const prNumber = (subject) => subject.match(/^Merge pull request #(\d+)/)?.[1] ?? subject.match(/\(#(\d+)\)$/)?.[1];

const readCommits = (previousTag, sourceSha) => execFileSync('git', ['log', '--first-parent', '--format=%s', `${previousTag}..${sourceSha}`], { encoding: 'utf8' })
  .split('\n').filter(Boolean).reverse().map((subject) => {
    const number = prNumber(subject);
    if (!number) return { subject };
    const pr = JSON.parse(execFileSync('gh', ['pr', 'view', number, '--repo', repo, '--json', 'number,title,body'], { encoding: 'utf8' }));
    return { subject, pr };
  });

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [version, previousTag, sourceSha] = process.argv.slice(2);
  if (!version || !previousTag || !sourceSha) throw new Error('Usage: release-notes.mjs <version> <previous-tag> <source-sha>');
  const notes = renderReleaseNotes({ version, previousTag, commits: readCommits(previousTag, sourceSha) });
  process.stdout.write(`${readFileSync(header, 'utf8').trimEnd()}\n\n${notes}`);
}
