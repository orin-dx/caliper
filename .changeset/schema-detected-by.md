---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
---

Real `extract` output now validates against the schema that `caliper-docgen schema` prints; components found by four of the detection rules, such as `taggedFactory`, used to fail validation because the schema did not list them. No action needed.
