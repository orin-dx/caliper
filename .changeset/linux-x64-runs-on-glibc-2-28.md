---
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
---

The Linux x64 binaries now run on systems with glibc 2.28 or newer, such as Debian 12, Ubuntu 22.04 and RHEL 9; they previously needed glibc 2.39 and failed to start on those systems. No action needed.
