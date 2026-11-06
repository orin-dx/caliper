---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With the experimental TypeScript checker on, a function prop keeps the optional and rest markers its parameters are written with. antd's `Progress.format` reads `(percent?: number, successPercent?: number) => ReactNode` and `ActionButton.close` reads `(...args: any[]) => void`; the checker's answer used to lose the markers, giving `(percent: undefined | number, …)` and `(args: any[]) => void`, which changes how a call site reads.
