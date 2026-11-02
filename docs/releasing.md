# Releasing

How a change becomes a published release, and what to check afterwards. Callisto 0.16.0 drives it (`callisto.toml`, `.github/workflows/release.yml`); its own guide is `docs/releasing.md` in `orin-dx/callisto`.

## Flow

1. A pull request adds a changeset (`.changeset/`, see `AGENTS.md`).
2. On every push to `main`, the `version-pr` job opens or refreshes the release pull request `callisto/version-packages`.
3. Merging that pull request is the approval. The `plan` job derives the release from the merged commit, and the `execute` job publishes.

`execute` is the only job with registry credentials.

## What a release produces

| Output | Where | Notes |
| --- | --- | --- |
| Crates | crates.io | `caliper-docgen-core`, `-tsc-client`, `-semantic`, `caliper-docgen` |
| npm packages | npm, with provenance | CLI wrapper and its five platform packages, napi binding and its platform packages, Vite plugin, Storybook addon |
| Tags | this repository | One per package, such as `cargo/caliper-docgen@x.y.z` and `npm/caliper-docgen@x.y.z` |
| GitHub Release | this repository | One per version, on the tag `cargo/caliper-docgen@x.y.z`; notes are the `## x.y.z` section of `crates/cli/CHANGELOG.md` |

Tag names do not change. The Homebrew tap reads the `npm/caliper-docgen@x.y.z` tag (`CONTRIBUTING.md`, Releases).

## Check a release

A green run is not proof of publication. After each release:

1. Open the run's `release-receipt` artifact, or the receipt in the job summary, and confirm every operation succeeded.
2. `gh release view cargo/caliper-docgen@x.y.z` shows the published Release and its notes.
3. `cargo info caliper-docgen` and `npm view caliper-docgen version` show the new version.

## Recover a failed release

- **A job failed:** "Re-run failed jobs". Callisto adopts the effects that already landed and does the rest.
- **E148, release PR snapshot changed:** the `version-pr` job planned from a release PR that changed before it applied the plan. Nothing was changed. Re-run the failed job, or push to `main` again.
