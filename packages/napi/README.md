# caliper-docgen-napi

Node.js bindings for [caliper-docgen](https://github.com/orin-dx/caliper), so JavaScript can call the extractor directly.

```bash
npm install caliper-docgen-napi
```

The prebuilt addon for your OS and CPU installs as an optional dependency (`caliper-docgen-napi-darwin-arm64`, `caliper-docgen-napi-linux-x64-gnu`, and so on).

Most projects want [`caliper-docgen-vite-plugin`](https://www.npmjs.com/package/caliper-docgen-vite-plugin) or the [`caliper-docgen`](https://www.npmjs.com/package/caliper-docgen) CLI, which use this package. The binding returns the canonical output format, not the `react-docgen-typescript` shape.

Docs and source: [github.com/orin-dx/caliper](https://github.com/orin-dx/caliper).

## License

FSL-1.1-MIT. Each version becomes MIT two years after its release.
