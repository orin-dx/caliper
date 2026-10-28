import type * as Napi from 'caliper-docgen-napi'

export interface SessionOptions {
  /** Absolute source directories. */
  srcDirs: string[]
  exclude?: string[]
  tsconfigPath?: string
  skipHtmlProps?: boolean
  htmlAttributes?: Napi.JsExtractOptions['htmlAttributes']
  /** Read and write the on-disk caches. Default: `true`. */
  cache?: boolean
  loadDependencies?: boolean
  collapseInheritedProps?: Napi.JsExtractOptions['collapseInheritedProps']
  /**
   * Resolve a component when a story asks for it rather than every component at start. Default: `true`. The answers
   * are the same; a session that serves a few components of many starts and holds less.
   */
  lazy?: boolean
  /**
   * After a lazy session starts, resolve a few components in the background so the dependency declarations and memos
   * the stories share are loaded before the first one asks. Default: `false`: measured against the same stories, it
   * did not make the first story faster, because the warm-up and the story compete for the same threads.
   */
  warmUp?: boolean
  /**
   * Once this many distinct components were asked for, resolve the rest in one background pass. Default: `0`, never:
   * it lowers the median story latency but raises peak memory by 170 to 420 MB.
   */
  fillAfter?: number
  /**
   * Ask TypeScript's 7.1 checker for each component a story names (experimental). `'off'` (the default) never starts
   * it; `'on-demand'` keeps one `tsc` warm, asks it for exactly the requested component, and takes the structural
   * props when it is slower than `semanticWaitMs`. Needs `typescript@beta` installed, or `tscPath`.
   */
  semanticTypes?: Napi.JsExtractOptions['semanticTypes']
  /** The `tsc` binary to run, instead of the nearest `node_modules/typescript/bin/tsc`. */
  tscPath?: string
  /** How many milliseconds a story waits for the checker before it takes the structural props. Default: `150`. */
  semanticWaitMs?: number
}

/** What the native session is created with: the session's options, with `lazy` decided by the caller. */
export function toNapiOptions(options: SessionOptions, lazy: boolean): Napi.JsExtractOptions {
  return {
    srcDirs: options.srcDirs,
    exclude: options.exclude,
    tsconfigPath: options.tsconfigPath,
    htmlAttributes: options.htmlAttributes ?? (options.skipHtmlProps ? 'none' : undefined),
    cache: options.cache,
    loadDependencies: options.loadDependencies,
    collapseInheritedProps: options.collapseInheritedProps,
    lazy,
    semanticTypes: options.semanticTypes,
    tscPath: options.tscPath,
    semanticWaitMs: options.semanticWaitMs,
  }
}
