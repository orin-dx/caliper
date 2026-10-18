## Type of change

- [ ] Bug fix
- [ ] New feature / pattern support
- [ ] Performance
- [ ] Refactor (no behavior change)
- [ ] Docs

## Why

What problem this solves or what pattern it adds support for — not a restatement of the diff.

## Test plan

- [ ] `cargo test -p caliper-docgen-core` passes
- [ ] `cargo clippy -p caliper-docgen-core -- -D warnings` passes
- [ ] Ran `cargo insta review` and accepted only intentional snapshot changes (if extraction output changed)
- [ ] `pnpm --filter @orin-dx/caliper-docgen-vite-plugin test` passes (if the Vite plugin or NAPI surface changed)

## Related

Closes #<issue>
