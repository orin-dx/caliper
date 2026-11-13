---
cargo/caliper-docgen-semantic: minor
---

For library users: `OnDemand::recycles()` reports how many times the warm checker was replaced on purpose (`worn`: it built its 64 programs). Replacements are also logged at debug level. Nothing changes for the CLI, the addon or the output.
