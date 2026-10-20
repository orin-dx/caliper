# caliper-docgen-napi

Native Node.js bindings for [caliper-docgen](https://github.com/orin-dx/caliper): React component prop extraction on OXC, callable from JavaScript.

```bash
npm install caliper-docgen-napi
```

The right prebuilt addon for your OS and CPU arrives as an optional dependency (`caliper-docgen-napi-darwin-arm64`, `-linux-x64-gnu`, and so on). Most projects want [`caliper-docgen-vite-plugin`](https://www.npmjs.com/package/caliper-docgen-vite-plugin) or the [`caliper-docgen`](https://www.npmjs.com/package/caliper-docgen) CLI instead, which use this package underneath.

The binding exposes this tool's own canonical output format, not the `react-docgen-typescript` shape.

Docs and source: https://github.com/orin-dx/caliper

License: AGPL-3.0-or-later.
