---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-storybook: minor
---

Breaking for Windows users: Windows is no longer a supported platform; use WSL. Caliper no longer builds or publishes `x86_64-pc-windows-msvc` binaries, and the `caliper-docgen-win32-x64-msvc` and `caliper-docgen-napi-win32-x64-msvc` npm packages are removed from the optional dependencies of `caliper-docgen` and `caliper-docgen-napi`, so installing on Windows no longer fetches a binary.
