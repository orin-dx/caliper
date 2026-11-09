---
cargo/caliper-docgen-semantic: minor
---

For library users: `OnDemand::recycles()` reports how many times the warm checker's process was replaced on purpose, split into `worn` (it built its 64 programs) and `bloated` (it passed `max_rss_mib`), and `OnDemand::process_id()` names the process that answered last. Replacements are also logged at debug level. Nothing changes for the CLI, the addon or the output.
