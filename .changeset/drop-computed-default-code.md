---
cargo/caliper-docgen-core: minor
---

Remove the `DiagnosticCode::ComputedDefault` variant and the `DefaultSource::JsDoc` variant from the Rust API. Nothing constructed either, so no output ever carried them; only Rust code naming them directly breaks.
