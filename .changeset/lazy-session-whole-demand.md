---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

- **A lazy session read across a project is up to 10× faster:** each growth of the dependency layer rebuilt the resolution context, once per new component. After the layer has grown three times, the rest of the project's dependency names now load once on a background thread. Fifty serial requests take 42 ms on Spectrum (were 240), 35 ms on antd (were 157), 77 ms on shadcn (were 254) and about 233 ms on Chakra (were 370), and a project read through the Storybook addon takes 0.4 s on Spectrum (was 1.1 s) and 0.8 s on Chakra (was 1.1 s). The first answer is unchanged and answers still equal a batch run's. A session that reads across a project holds all of its dependency names: shadcn's peak memory after 50 requests goes from 122 to 192 MB.
- **`WatchSession::wait_for_dependency_load`:** waits for that background load, for benchmarks and tests that want a settled session. A request never needs it.
