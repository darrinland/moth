# Moth: working rules for Claude

## Before every commit

- Run the tests locally before committing. CI is in the backlog, so the local run is the only
  gate for now. Run these from the repository root and all must pass:
  - `cargo fmt --all --check`
  - `cargo test --workspace`
- Run `cargo clippy --all-targets` too, and report any new warnings. Existing warnings are
  tracked in `docs/PLAN.md`, so do not fix them as a side effect of an unrelated change.
- If a command fails, do not commit. Fix the cause, or tell the user what failed and why it
  cannot be fixed now.
- If cargo is not available in the shell, say so and do not commit. Do not skip the check.
- Do not claim tests passed unless their output was read in this session.

## After every commit

- Push to `origin` as soon as the commit is made. Do not leave commits unpushed.

## Scope

- Targets for 1.0 are macOS (Apple Silicon) and Windows (x64). Do not add Linux-specific work
  without asking; see `docs/PLAN.md`.

## Testing the app

- Run the self-test with `MOTH_SMOKE=<dir> ./target/release/moth-ui`. It opens off screen and
  never takes focus (macOS accessory policy), so it is safe to run while the user works. Do not
  add `with_always_on_top`, window raising or `osascript` activation to it.
