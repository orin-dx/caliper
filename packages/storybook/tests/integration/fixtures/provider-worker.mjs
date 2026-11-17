// Runs the built provider inside a worker thread, as Storybook's docgen worker does: each message is one docgen request.
// `workerData` holds options added to the defaults below.
import { parentPort, workerData } from 'node:worker_threads'
import { createDocgenProvider } from '../../../dist/provider.js'

const provider = (await createDocgenProvider({ srcDirs: ['src'], ...workerData }))(async () => ({ id: 'next' }))

parentPort.on('message', async (entry) => {
  // oxlint-disable-next-line unicorn/require-post-message-target-origin -- a worker port has no origin
  parentPort.postMessage(await provider({ entry }))
})
