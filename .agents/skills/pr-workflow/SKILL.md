---
name: pr-workflow
description: Deliver every beeper-rs change as a small pull request into main, with the project design rules, the local checks, UI screenshot comparison, and ASD-STE100 commit and PR text. Use for any code change, commit, branch, pull request, or merge in this repository.
---

# beeper-rs pull request workflow

## Design philosophy

I value robustness, minimizing total lines of code, avoiding bloat, avoiding duplication.
I hate fallbacks, useless if-else statements (things like "elif code is broken then repair it" should not exist, it should fail loudly), and I love asserts, which allow to catch early bugs. Fallbacks hide bugs and bloat the code.

In practice:

- Keep the code short and concise. Do not add features that the task does not ask for.
- Use `assert!`, `expect("why this holds")`, and `panic!` for broken invariants. Do not add `unwrap_or_default()`, silent `continue`, or "repair" branches for states that must not occur.
- Return `Result` only for errors that are not bugs: network, user input, and files from other programs. Show these errors to the user.
- Before you add a helper, search for one that already exists (`rg`). Extend it. Do not copy it.
- Prefer data tables (for example `crates/beeper-core/src/prefs.rs`) to repeated code.
- Delete dead code, unused fields, and stale comments in the same PR.
- Add a dependency only when it removes more code than it adds. Disable default features that the code does not use.

## References

- Copy the layout, the text, and the behavior of Beeper Desktop. Use the screenshots in `tmp/` and the JavaScript in the Beeper Desktop `app.asar`.
- Use the `github.com/beeper` repositories (for example `desktop-api-js` and `static`) for names, structure, and assets.

## One PR for each change

1. Start from the latest `main`: `git fetch origin && git switch -c <type>/<topic> origin/main`.
2. Keep one subject for each PR. Put unrelated changes on a different branch.
3. Run the checks (all must pass):
   ```bash
   cargo fmt --all
   cargo clippy --locked --workspace --all-targets -- -D warnings
   cargo test --locked --workspace -- --nocapture
   ```
4. For UI changes, render the screens and compare them with the Beeper Desktop reference screenshots in `tmp/`:
   ```bash
   cargo run -q --example shots -- /tmp/shots dark
   cargo run -q --example shots -- /tmp/shots-light
   ```
5. Commit with Conventional Commits. Write one short sentence for each message, with no body. The `.githooks/commit-msg` hook rejects other formats (`git config core.hooksPath .githooks`).
6. Push and open the PR: `git push -u origin HEAD && gh pr create --base main --title "<commit subject>" --body-file <file>`.
7. Wait for CI: `gh pr checks --watch`. Fix all failures on the same branch.
8. Merge, then update the local `main`: `gh pr merge --squash --delete-branch && git switch main && git pull --ff-only`.
9. Start the next change from step 1.

## ASD-STE100 text

Commit subjects, PR titles, PR bodies, and README text use ASD-STE100 Simplified Technical English:

- Use short sentences: 20 words or fewer for instructions, 25 words or fewer for descriptions.
- Write one instruction in each sentence. Use the imperative for instructions.
- Use the active voice and the simple tenses (present, past, future).
- Use one word for one meaning. Do not use synonyms for the same item.
- Do not use "-ing" words as nouns or adjectives when a simple form is possible.
- Use articles ("the", "a") where English permits them.
- Do not add tool attribution trailers.

PR body template:

```markdown
## Changes

- <What the PR changes. One item for each change.>

## Reason

<Why the change is necessary.>

## Test

- <The commands that you ran and the result.>
```

Example commit subject: `fix(ui): show the full chat header in empty chats`.
