Update and review snapshot tests.

```bash
INSTA_UPDATE=always cargo test -p caliper-docgen-core 2>&1
cargo insta review
```

After `cargo insta review` completes, run `cargo test -p caliper-docgen-core` once more to confirm all snapshots pass.
