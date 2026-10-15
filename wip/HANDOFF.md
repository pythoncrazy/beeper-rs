# Handoff state (2026-10-15)

Read `.agents/skills/pr-workflow/SKILL.md` first. All earlier work is on `main` (PRs #30 and #32 to #37). No branch is open.

## Open items

1. **Rail logo**: the hungryserv (Beeper) account uses the Matrix "m" logo. The Beeper logo is in `beeper/static` (`brand/Beeper_Adaptive_Icon_108dp.svg`), but that repository has no license. Simple Icons has no Beeper logo. Ask the user before you copy it.
2. **Flaky Xvfb screenshot**: the e2e test sometimes fails with `/tmp/beeper_rs_xvfb_window.png is too small` (PR #34, first run). A rerun passed. `UiMessage::WindowOpened` takes the screenshot at once. The X11 window probably does not have its final size yet.
3. **Chat list status icon**: Beeper shows a dashed circle for messages that are not delivered. The message model has no delivery or read state.

## This sandbox

- Install fontTools for `icons.py` with `PYTHONPATH=<fonttools wheel>`. There is no pip.
- `/tmp` is `noexec`. Extract the Beeper AppImage in `target/beeper/`.
- The sandbox blocks the GitHub GraphQL API. Use REST: `gh api repos/pythoncrazy/beeper-rs/pulls` to open a PR, `gh api -X PUT .../pulls/N/merge -f merge_method=squash` to merge, and `gh api -X DELETE .../git/refs/heads/<branch>` to delete the branch.
- weston and Xvfb are not installed, so the local e2e test stops at `spawn weston`. CI runs these steps.
- `export CARGO_BUILD_JOBS=20`.
