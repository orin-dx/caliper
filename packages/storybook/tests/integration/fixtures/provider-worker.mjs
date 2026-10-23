// Runs the built provider inside a worker thread, as Storybook's docgen worker does: each message is one docgen request.
import { parentPort } from 'node:worker_threads'
import { createDocgenProvider } from '../../../dist/provider.js'

const provider = (await createDocgenProvider({ srcDirs: ['src'] }))(async () => ({ id: 'next' }))

parentPort.on('message', async (entry) => {
  // oxlint-disable-next-line unicorn/require-post-message-target-origin -- a worker port has no origin
  parentPort.postMessage(await provider({ entry }))
})
