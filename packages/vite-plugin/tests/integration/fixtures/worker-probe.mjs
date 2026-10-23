// Runs inside a worker thread: loads the native binding, extracts once, then opens a watch session and initializes it.
import { parentPort, workerData } from 'node:worker_threads'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const napi = require('caliper-docgen-napi')
const options = { srcDirs: [workerData.srcDir] }
const cold = JSON.parse(await napi.extractAll(options))
const sessionId = napi.createSession(options)
const session = JSON.parse(await napi.initializeSession(sessionId, options))
napi.closeSession(sessionId)
// oxlint-disable-next-line unicorn/require-post-message-target-origin -- a worker port has no origin
parentPort.postMessage({ cold: Object.keys(cold.components), session: Object.keys(session.components), sessionId })
