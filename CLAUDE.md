<!-- entire-agent:begin -->
Read .entire/agent-guide.md for this repository's workflow, source inspection, and verification guidance.
@.entire/agent-guide.md
<!-- entire-agent:end -->

## Working as an agent in this repository

Rules for any agent (lead or worker) changing the repo. The brief names the task, the files you own and a tool-call cap; these rules do not repeat in briefs.

- **Scope.** Edit only files the task names; if another must change, stop and say which and why. Do not ask the user questions; if blocked, report what blocks and what you tried. Do not wait on background work.
- **Toolchain.** Rust 1.99 or newer (`rust-version` in `Cargo.toml`). `mise` may force 1.98.1: build with `/Users/gabe/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin` first on `PATH` and commit with `MISE_RUST_VERSION=1.99.1` (the pre-commit hook uses it). With that `bin` first, `rust-objcopy` fails and `strip` silently does nothing: irrelevant unless you measure binary size. Use your own `CARGO_TARGET_DIR`; scratch lives outside the repo, never in `/tmp`.
- **Git.** Branch from `origin/main`. Signed commits ending `Claude-Session: <session url>`. Push normally (`git push origin HEAD:<branch>`), never `--no-verify`, then check `git ls-remote`. Merge `main` into a branch (merge commit); never rebase a pushed branch or force-push. Check that a merged PR's content is really on `main` (`git cat-file -e origin/main:<path>`): a PR merged into its base branch is not on `main`.
- **Checks are targeted; CI runs the rest.** Always `just lint doc-check machete typos` before pushing. Then only what the change touches: Rust, nextest of the touched crates; types, output or merge, `just test-checker-lane`; checker or session code, the `gate_harness` determinism and memory modes; an output-preserving change, a full-content diff (JSON parsed, `stats` dropped) of `extract --json --quiet --no-cache` against a build of `origin/main` on a large and a Chakra-only project. `deny`, `semver`, `test-ts`, `test-napi`, `test-integration` and `test-py` run in CI: run them locally only if you changed that area.
- **Tests fail on base** for every behaviour change. No oracle runs (`scripts/weekly.py --oracle`) and no corpus fetches: they cost money and are the owner's call. No new scripts, options or recipes unless CI or the weekly calls them; delete what a change makes dead in the same PR.
- **Timing.** The machine is shared: check `uptime`, and prefer allocation or instruction counts (load-independent). Take wall-clock A/Bs only when asked, and report the load.
- **Open PRs.** Do not open or edit the PR and do not poll CI: the lead links the branch to its trail (Entire opens the PR) and publishes your PR body.
- **Artifacts, agent to agent.** Write the PR body to a file the lead names: `## What`, `## Evidence` (numbers, exit codes, what was diffed), `## Not verified`, then the session URL. Your final message is at most 120 words: branch, verified head SHA, body path, blockers. Stop at the tool-call cap and report what you have verified; "not done, because" beats burning more calls.
