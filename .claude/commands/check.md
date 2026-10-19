Run the verification suite and report status. Do not fix anything.

```bash
cargo clippy -p caliper-docgen-core -- -D warnings 2>&1
cargo test -p caliper-docgen-core 2>&1
cargo insta pending-snapshots 2>/dev/null && echo "⚠ pending snapshots" || echo "✓ no pending snapshots"
```

Report: warnings, failing tests, pending snapshots. Stop there.
