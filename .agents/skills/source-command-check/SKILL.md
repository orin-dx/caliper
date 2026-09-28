---
name: "source-command-check"
description: "Migrated source command `check`"
---

# source-command-check

Use this skill when the user asks to run the migrated source command `check`.

## Command Template

Run the verification suite and report status. Do not fix anything.

```bash
cargo clippy -p oxc-react-docgen-core -- -D warnings 2>&1
cargo test -p oxc-react-docgen-core 2>&1
cargo insta pending-snapshots 2>/dev/null && echo "⚠ pending snapshots" || echo "✓ no pending snapshots"
```

Report: warnings, failing tests, pending snapshots. Stop there.
