---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

- **Breaking (canonical JSON):** each distinct prop is written once, in a top-level `props` table, and a component's `props` and `notableInherited` map each prop name to an id (16 hex digits). Read a prop as `output.props[output.components[key].props[name]]`. The same prop has the same id in every output and update of a build. Mantine's output drops from 19.3 MB to 3.6 MB and parses 7× faster. This covers `extract --json`, `--format canonical`, the NAPI functions and the Vite plugin's virtual module; `--format rdt` and `--format storybook` keep props inline.
- **Breaking (watch updates):** an update carries `props` with every prop its `updatedComponents` refer to. Merge it into the output's table; ids no component refers to can be dropped. The Vite plugin's `caliper-docgen:update` event carries it too.
