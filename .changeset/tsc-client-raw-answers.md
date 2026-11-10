---
cargo/caliper-docgen-tsc-client: minor
---

For library users: `Client::request_raw` answers with a `RawResponse`, the response's `result` as the JSON text it arrived as, so a caller that knows the shape it expects parses it once, straight into that shape, instead of first into a `serde_json::Value`. `Client::request` answers as before. Frames are now read in one pass, which makes every answer cheaper to receive; caliper's checker backend uses the raw form, and on Chakra's `Button` (852 props) its first answer is about 25% faster.
