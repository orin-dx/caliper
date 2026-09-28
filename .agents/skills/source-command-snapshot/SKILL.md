---
name: "source-command-snapshot"
description: "Migrated source command `snapshot`"
---

# source-command-snapshot

Use this skill when the user asks to run the migrated source command `snapshot`.

## Command Template

Update and review snapshot tests.

```bash
INSTA_UPDATE=always cargo test -p oxc-react-docgen-core 2>&1
cargo insta review
```

After `cargo insta review` completes, run `cargo test -p oxc-react-docgen-core` once more to confirm all snapshots pass.
