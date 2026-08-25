# Issue tracker: GitHub

Issues and PRDs for this repo live as GitHub issues. Use the `gh` CLI for all operations.

## Conventions

- **Create an issue**: write the exact Markdown to a temporary body file, then
  run `gh issue create --title "..." --body-file <path>`.
- **Read an issue**: `gh issue view <number> --comments`, filtering comments by `jq` and also fetching labels.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Comment on an issue**: `gh issue comment <number> --body-file <path>`.
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close with a comment**: post the comment through `--body-file`, verify it,
  then run `gh issue close <number>`.

Infer the repo from `git remote -v` — `gh` does this automatically when run inside a clone.

## Identity and safe writes

On a machine with multiple GitHub accounts, list the authenticated accounts
with `gh auth status --hostname github.com`. For a personal repository, try the
account matching the repository owner first; for an organization, choose an
account with repository access. Obtain its token with `gh auth token -u
<account>`, verify `GH_TOKEN=<token> gh api user -q .login`, and pass that
`GH_TOKEN` to every read or write command without printing or persisting it.
Abort on an identity or repository-access mismatch. Never rely on the ambient
active account.

Treat Markdown as data. Use `--body-file`, standard input, or a structured API
payload, and use a quoted heredoc only when producing a file with no shell
interpolation. For dynamic issue identifiers, insert explicit placeholders
through a narrowly controlled substitution step after the body is created.
Never place Markdown backticks or dollar substitutions directly inside a shell
command string. Read every created or edited issue/comment back and verify its
body, labels, and relationships before reporting success.

When changing a triage state, remove every other configured state label and add
the intended state, then read the labels back and assert that exactly one state
role and one category role remain.

## Sub-issues and dependencies

Use GitHub's typed REST fields for relationship mutations. `gh api -F` converts
numeric values to JSON integers; `-f` sends strings and must not be used for
`sub_issue_id` or `issue_id`.

- **List sub-issues**: `gh api --paginate repos/{owner}/{repo}/issues/{parent_number}/sub_issues`
- **Add an existing sub-issue**: first resolve the child issue's database ID,
  then run `gh api -X POST repos/{owner}/{repo}/issues/{parent_number}/sub_issues -F sub_issue_id=<child_database_id>`
- **List blockers**: `gh api --paginate repos/{owner}/{repo}/issues/{issue_number}/dependencies/blocked_by`
- **Add a blocker**: first resolve the blocking issue's database ID, then run
  `gh api -X POST repos/{owner}/{repo}/issues/{issue_number}/dependencies/blocked_by -F issue_id=<blocker_database_id>`

After either mutation, rerun the corresponding list command and assert that the
returned issue database ID and number match the intended relationship.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

When set to `yes`, PRs run through the same labels and states as issues, using the `gh pr` equivalents:

- **Read a PR**: `gh pr view <number> --comments` and `gh pr diff <number>` for the diff.
- **List external PRs for triage**: `gh pr list --state open --json number,title,body,labels,author,authorAssociation,comments` then keep only `authorAssociation` of `CONTRIBUTOR`, `FIRST_TIME_CONTRIBUTOR`, or `NONE` (drop `OWNER`/`MEMBER`/`COLLABORATOR`).
- **Comment / label / close**: `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`.

GitHub shares one number space across issues and PRs, so a bare `#42` may be either — resolve with `gh pr view 42` and fall back to `gh issue view 42`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body. `gh issue create --label wayfinder:map`.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue (`gh api` on the sub-issues endpoint). Where sub-issues aren't enabled, add the child to a task list in the map body and put `Part of #<map>` at the top of the child body. Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: GitHub's **native issue dependencies** — the canonical, UI-visible representation. Add an edge with `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker's numeric **database id** (`gh api repos/<owner>/<repo>/issues/<n> --jq .id`, _not_ the `#number` or `node_id`). GitHub reports `issue_dependencies_summary.blocked_by` (open blockers only — the live gate). Where dependencies aren't available, fall back to a `Blocked by: #<n>, #<n>` line at the top of the child body. A ticket is unblocked when every blocker is closed.
- **Frontier query**: list the map's open children (`gh issue list --state open`, scoped to the map's sub-issues / task list), drop any with an open blocker (`issue_dependencies_summary.blocked_by > 0`, or an open issue in the `Blocked by` line) or an assignee; first in map order wins.
- **Claim**: `gh issue edit <n> --add-assignee @me` — the session's first write.
- **Resolve**: `gh issue comment <n> --body "<answer>"`, then `gh issue close <n>`, then append a context pointer (gist + link) to the map's Decisions-so-far.
