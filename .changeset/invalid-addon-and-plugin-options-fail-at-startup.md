---
npm/caliper-docgen-storybook: patch
npm/caliper-docgen-vite-plugin: patch
---

An invalid option value now fails at startup with a message naming the choices, such as `fallback: 'degrade'` in the addon or `logLevel: 'verbose'` in the Vite plugin; a typo used to log everything or act as `fallback: 'never'`. Check your options if the build now stops with a `[caliper-docgen]` message.
