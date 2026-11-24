# Beam: project rules

These rules apply to every change in this repository. They override global defaults.

## GitHub account
- Use the **Ayush-e4** GitHub account for everything in this repo: pushes, PRs, merges, comments.
- Never use `ayushtxp` here, and never run `gh auth switch` (another session on this PC relies on
  `ayushtxp` being the active account). Instead, prefix `gh` commands with
  `GH_TOKEN=$(gh auth token --user Ayush-e4)`. Never print the token.
- The `origin` remote is pinned to `https://Ayush-e4@github.com/Ayush-e4/beam.git`, so plain
  `git push`/`git fetch` already authenticate as Ayush-e4.
- Commits use the repo-local identity (`Ayush-e4`). Do not change it.
- Never mention Claude or AI tools in commits, PR titles/bodies, or comments. No `Co-Authored-By`
  trailers.

## Branches and pull requests
- `master` is the only long-lived branch. Never commit directly to it.
- Every change goes through a short-lived branch and a pull request, which is then squash-merged.
- Branch names: `feat/...`, `fix/...`, `chore/...`, `docs/...`, `test/...`, `refactor/...`.
- One logical change per PR. Keep PRs small and focused.
- The repo deletes branches automatically on merge; also delete the local branch after merging.
- Experimental work (e.g. a future v2) gets its own branch and follows the same rules.

## Code rules
- **No file longer than 300 lines.** Split modules before they reach the limit.
- Use the latest **stable / LTS** versions of languages, SDKs and libraries. If a version is very
  new or its status is uncertain, verify it before adopting, and prefer the latest LTS.
- Code must be readable: clear names, small functions, comments only where intent isn't obvious.
- Every file is auto-formatted with the language's standard formatter before committing.
- Follow each language's idiomatic best practices and lint with zero warnings.
- Handle errors explicitly: no swallowed exceptions, no `unwrap()`/`!!` on fallible paths,
  meaningful error types and messages, and failures surfaced to the user where relevant.
- Keep a clear folder structure; each module has one responsibility.

## Testing
- New behaviour ships with tests. Bug fixes ship with a test that reproduces the bug.
- All tests and linters must pass before a PR is merged.

## When unsure
- Ask before making product decisions, adding major dependencies, or anything hard to reverse.
