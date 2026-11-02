# Releasing

How a change becomes a published release, and what to check afterwards. Callisto 0.16.0 drives it (`callisto.toml`, `.github/workflows/callisto-release.yml`); its own guide is `docs/releasing.md` in `orin-dx/callisto`.

## Flow

1. A pull request adds a changeset (`.changeset/`, see `AGENTS.md`).
2. On every push to `main`, the `version-pr` job opens or refreshes the release pull request `callisto/version-packages`.
3. Merging that pull request is the approval. The jobs then run in this order:

| Job | What it does | Credentials |
| --- | --- | --- |
| `plan` | Derives the release from the merged commit; records whether it has CLI assets | none |
| `build` | Builds the napi addons and CLI binary for each of five targets; archives, attests and uploads the CLI asset | `id-token`, `attestations: write` |
| `assemble` | Writes the artifact manifest that binds the assets to the plan | none |
| `execute` | Publishes crates and npm packages, tags, uploads the assets and publishes the GitHub Release; verifies every attestation first | registry tokens |
| `drain` | Starts the next unfinished release, if any, once `execute` succeeds | `actions: write` |

`execute` is the only job with registry credentials. `assemble` is skipped when the plan declares no assets (`has_artifacts` is false).

The workflow runs on every push to `main`, daily at 05:23 UTC, and on manual dispatch. Every run first asks `callisto release pending` for the oldest merged release commit whose tags or GitHub Release are missing, and finishes that one before anything newer. When none is unfinished, a push to a release commit releases it and any other run refreshes the release pull request.

The workflow file must stay named `callisto-release.yml`. Callisto verifies each asset's attestation against that exact workflow path (`gh attestation verify --signer-workflow <repo>/.github/workflows/callisto-release.yml`), and the path is fixed in Callisto 0.16.0.

## What a release produces

| Output | Where | Notes |
| --- | --- | --- |
| Crates | crates.io | `caliper-docgen-core`, `-tsc-client`, `-semantic`, `caliper-docgen` |
| npm packages | npm, with provenance | CLI wrapper and its five platform packages, napi binding and its platform packages, Vite plugin, Storybook addon |
| Tags | this repository | One per package, such as `cargo/caliper-docgen@x.y.z` and `npm/caliper-docgen@x.y.z` |
| GitHub Release | this repository | One per version, on the tag `cargo/caliper-docgen@x.y.z`; notes are the `## x.y.z` section of `crates/cli/CHANGELOG.md` |
| CLI binaries | the GitHub Release | `caliper-docgen-<target>.tar.gz` for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`; each holds the binary at the archive root |

Tag names do not change. The Homebrew tap reads the `npm/caliper-docgen@x.y.z` tag (`CONTRIBUTING.md`, Releases).

The build matrix in `callisto-release.yml` and the `[[release.artifact]]` entries in `callisto.toml` list the same five targets and asset names. Change both together.

## Check a release

A green run is not proof of publication. After each release:

1. Open the run's `release-receipt-attempt-N` artifact, or the receipt in the job summary, and confirm every operation succeeded.
2. `gh release view cargo/caliper-docgen@x.y.z` shows the published Release, its notes and five assets.
3. `gh attestation verify <asset> --repo orin-dx/caliper --signer-workflow orin-dx/caliper/.github/workflows/callisto-release.yml` passes for a downloaded asset.
4. `cargo info caliper-docgen` and `npm view caliper-docgen version` show the new version.

## Recover a failed release

A release that stopped is picked up by the next push to `main` or the daily run, and `drain` chains any further unfinished ones. While one is unfinished, the release pull request is not refreshed, and a newer release pull request published first would release without any warning that the older version was never tagged. Finish the older release first.

List what is unfinished, oldest first, with what is missing:

```sh
callisto release pending --from 2daef9276b234fcc04d8994d91210eeccd88d7e8
```

`--from` is the `RELEASE_PENDING_FROM` value in `callisto-release.yml`. Without it, `release pending` reads the committed decision of the 10 newest release commits and stops with E166 on one whose decision no longer decodes. The pin is the 0.9.0 release commit (`2daef927`).

0.9.0 was published before GitHub Releases were enabled, so `release pending` lists it as missing its GitHub Release. The first run after this flow lands finishes it: the build matrix rebuilds 0.9.0 from its release commit and Callisto creates the GitHub Release with five attested assets. Callisto adopts the crates, npm packages and tags that already landed (`docs/releasing.md` in `orin-dx/callisto`, "Publication checks"); this path has not been run, so watch that run.

- **A job failed:** "Re-run failed jobs". Callisto adopts the effects that already landed and does the rest.
- **A release cannot finish:** dispatch it by hand with its release commit, `gh workflow run callisto-release.yml --ref main -f release_source_sha=<merged release commit>`. If it will never finish, skip it by moving `RELEASE_PENDING_FROM` to a newer release commit.
- **E148, release PR snapshot changed:** the `version-pr` job planned from a release PR that changed before it applied the plan. Nothing was changed. Re-run the failed job, or push to `main` again.
- **`ArtifactAssetDiffers`:** a rebuilt asset is not byte-identical to the one already on the draft Release. Delete the mismatched asset, then dispatch the workflow again with the release commit:

  ```sh
  gh release delete-asset "cargo/caliper-docgen@x.y.z" "<asset>" --yes
  gh workflow run callisto-release.yml --ref main -f release_source_sha=<merged release commit>
  ```

- **E180, the workflow cannot push a tag:** GitHub rejects a `GITHUB_TOKEN` tag push when the tagged commit's workflows differ from every branch tip, which happens when recovering an older release. Push each tag of that release yourself with a personal token or deploy key, then dispatch as above.
- **E140, attestation verification failed:** `gh attestation verify` rejected an asset: wrong repository, signer workflow, signer digest, source digest, or a self-hosted runner. The signer digest is the commit that ran `plan` (`--orchestration-revision`), so recover by re-running the whole workflow so `plan`, `build` and `execute` share one revision.

## Rehearse a change to the release flow

Nothing in CI exercises the release path before a real release, so rehearse before merging a change to `callisto-release.yml` or the `[release]` section of `callisto.toml`:

1. Fork the repository, or create a scratch one, and enable Actions. Use the same file name for the workflow.
2. Set `forge-repository` in `callisto.toml` to the scratch repository; Callisto stops with E187 when it differs from `origin`. The scratch repository has no registry tokens, so expect `execute` to stop at its first publish. The point is that `plan`, `build` and `assemble` pass.
3. Add a changeset, merge the release pull request, and check that `build` produced five attested assets and `assemble` wrote `manifest.json`.
4. Read `callisto release --dry-run --format text` on the release commit to see the operations `execute` will run.
5. Confirm the first real release from the receipt and the providers, as in "Check a release".
