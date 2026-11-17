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
   * Ask the built-in TypeScript checker for each component a story names. Default: `true`: it is kept warm, asked for
   * exactly the requested component, and the structural props stand when it is slower than `semanticWaitMs`. `false`
   * never starts it.
   */
  checker?: boolean
  /** How many milliseconds a story waits for the checker before it takes the structural props. Default: `150`. */
  semanticWaitMs?: number
  /**
   * How many milliseconds a story waits for the checker when caliper-docgen's own pass lists no props for its component.
   * Default: `2000`.
   */
  semanticWaitEmptyMs?: number
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
    checker: options.checker,
    semanticWaitMs: options.semanticWaitMs,
    semanticWaitEmptyMs: options.semanticWaitEmptyMs,
  }
}
