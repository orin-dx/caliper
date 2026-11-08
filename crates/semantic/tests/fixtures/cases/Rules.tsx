import * as React from 'react';
import type { DepStyle, DepTheme, DepSize, Tone } from 'fake-dep';

// One component per mapping rule that Cases.tsx does not reach. Each takes the prop it is about, and `ok`.

/** Scalars and the checker's own spellings of them. */
export function Scalars(props: { s: string; n: number; b: boolean; big: bigint; sym: symbol; obj: object; nul: null; und?: undefined; nev: never; unk: unknown; any: any; vd: () => void }) { return null; }

/** Literals of every kind. */
export function Literals(props: { str: 'a'; num: -3; yes: true; no: false; big: 10n; neg: -1n; mixed: 'x' | 2 | true | null }) { return null; }

/** Enums: string and numeric, a member, a union of members, and the whole enum. */
export enum Size { Small = 'small', Large = 'large' }
export enum Level { Low, High = 5 }
export function Enums(props: { size: Size; member: Size.Small; two: Size.Small | Level.High; level: Level }) { return null; }

/** Template literals: a pattern that stays one, and one the checker expands. */
export function Templates(props: { pattern: `${number}px`; two: `--${string}-${number}`; expanded: `${'a' | 'b'}-${1 | 2}`; upper: Uppercase<string> }) { return null; }

/** Tuples of every element kind. */
export function Tuples(props: { labeled: [head: string, tail?: number, ...rest: boolean[]]; variadic: [...string[], number]; nested: [[number, string?], ...[1, 2]]; empty: []; readonlyLabeled: readonly [a: 1, b: 2] }) { return null; }

/** Objects: optional, readonly, methods, accessors, index signatures, call and construct signatures. */
export interface Shape {
  readonly id: string;
  label?: string;
  method(a: number): string;
  get computed(): number;
  [key: string]: unknown;
}
export function Objects(props: {
  shape: Shape;
  withCall: { (a: string): number; displayName: string };
  ctor: { new (a: string): Date };
  overloaded: { (a: string): string; (a: number): number };
  symbolKeyed: { [Symbol.iterator]: () => void; plain: 1 };
  numeric: { [n: number]: string };
}) { return null; }

/** Aliases: a local one expands, a dependency's object keeps its name, its literal union expands. */
type LocalPoint = { x: number; y: number };
export function Aliases(props: { local: LocalPoint; style: DepStyle; theme: DepTheme; size: DepSize; tone: Tone; partial: Partial<LocalPoint>; record: Record<'a' | 'b', number> }) { return null; }

/** Lib and dependency interfaces keep their name and arguments; React terminals are found by where they are declared. */
export function Named(props: {
  date: Date;
  promise: Promise<string>;
  map: Map<string, number[]>;
  node: React.ReactNode;
  element: React.ReactElement;
  css: React.CSSProperties;
  as: React.ElementType;
  ref: React.Ref<HTMLDivElement>;
  handler: React.MouseEventHandler<HTMLButtonElement>;
}) { return null; }

/** Recursion by alias and by interface, and mutual recursion. */
type Json = string | number | boolean | null | Json[] | { [key: string]: Json };
interface Left { right?: Right; name: string }
interface Right { left?: Left; id: number }
export function Recursion(props: { json: Json; left: Left }) { return null; }

/**
 * Recursion has to end, and an instantiation is not its declaration: an alias that is an array or a tuple of itself,
 * `Box<Box<string>>` (two instantiations of one declaration, the inner one finite), a generic that names itself, a
 * generic mutual pair, and a generic that names a new instantiation each time (which only the depth cap ends).
 */
type LoopArray = LoopArray[];
type LoopTuple = [LoopTuple];
type ReadonlyLoop = readonly ReadonlyLoop[];
interface Box<T> { value: T }
interface Pair<T, U> { first: T; second: U }
interface Tree<T> { value: T; kids: Tree<T>[] }
interface Ping<T> { pong?: Pong<T> }
interface Pong<T> { ping?: Ping<T> }
interface Grow<T> { next?: Grow<T[]>; item: T }
type Point = { x: number };
export function Instances(props: {
  loopArray: LoopArray;
  loopTuple: LoopTuple;
  readonlyLoop: ReadonlyLoop;
  boxed: Box<Box<string>>;
  twice: Pair<Point, Point>;
  nestedPair: Pair<Pair<Point, Point>, Point>;
  tree: Tree<string>;
  mutual: Ping<number>;
  grow: Grow<string>;
}) { return null; }

/**
 * The caps, one prop for each place a type is cut. A cut type is `truncated` with the limit and, for a width, how many
 * there were: depth 5 (an object, union, intersection, array, tuple, template literal), 100 fields at the top and 40
 * below it, 500 members at the top and 120 below it (a union, an intersection, a tuple).
 */
type Deep = { a: { b: { c: { d: { e: { f: { g: string } } } } } } };
interface Wide {
  f001: 1; f002: 2; f003: 3; f004: 4; f005: 5; f006: 6; f007: 7; f008: 8; f009: 9; f010: 10; f011: 11; f012: 12; f013: 13; f014: 14; f015: 15; f016: 16; f017: 17; f018: 18; f019: 19; f020: 20; f021: 21; f022: 22; f023: 23; f024: 24; f025: 25; f026: 26; f027: 27; f028: 28; f029: 29; f030: 30; f031: 31; f032: 32; f033: 33; f034: 34; f035: 35; f036: 36; f037: 37; f038: 38; f039: 39; f040: 40; f041: 41; f042: 42;
}
interface WideTop {
  f001: 1; f002: 2; f003: 3; f004: 4; f005: 5; f006: 6; f007: 7; f008: 8; f009: 9; f010: 10; f011: 11; f012: 12; f013: 13; f014: 14; f015: 15; f016: 16; f017: 17; f018: 18; f019: 19; f020: 20; f021: 21; f022: 22; f023: 23; f024: 24; f025: 25; f026: 26; f027: 27; f028: 28; f029: 29; f030: 30; f031: 31; f032: 32; f033: 33; f034: 34; f035: 35; f036: 36; f037: 37; f038: 38; f039: 39; f040: 40; f041: 41; f042: 42; f043: 43; f044: 44; f045: 45; f046: 46; f047: 47; f048: 48; f049: 49; f050: 50; f051: 51; f052: 52; f053: 53; f054: 54; f055: 55; f056: 56; f057: 57; f058: 58; f059: 59; f060: 60; f061: 61; f062: 62; f063: 63; f064: 64; f065: 65; f066: 66; f067: 67; f068: 68; f069: 69; f070: 70; f071: 71; f072: 72; f073: 73; f074: 74; f075: 75; f076: 76; f077: 77; f078: 78; f079: 79; f080: 80; f081: 81; f082: 82; f083: 83; f084: 84; f085: 85; f086: 86; f087: 87; f088: 88; f089: 89; f090: 90; f091: 91; f092: 92; f093: 93; f094: 94; f095: 95; f096: 96; f097: 97; f098: 98; f099: 99; f100: 100; f101: 101;
}
type Digit = 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9;
export function Caps(props: {
  deep: Deep;
  wide: { inside: Wide };
  wideTop: WideTop;
  many: `${Digit}${Digit}${0 | 1 | 2 | 3 | 4 | 5}`;
  nestedMany: { inside: `${Digit}${Digit}${0 | 1}` };
  allOf: { k1: 1 } & { k2: 2 } & { k3: 3 } & { k4: 4 } & { k5: 5 } & { k6: 6 } & { k7: 7 } & { k8: 8 } & { k9: 9 } & { k10: 10 } & { k11: 11 } & { k12: 12 } & { k13: 13 } & { k14: 14 } & { k15: 15 } & { k16: 16 } & { k17: 17 } & { k18: 18 } & { k19: 19 } & { k20: 20 } & { k21: 21 } & { k22: 22 } & { k23: 23 } & { k24: 24 } & { k25: 25 } & { k26: 26 } & { k27: 27 } & { k28: 28 } & { k29: 29 } & { k30: 30 } & { k31: 31 } & { k32: 32 } & { k33: 33 } & { k34: 34 } & { k35: 35 } & { k36: 36 } & { k37: 37 } & { k38: 38 } & { k39: 39 } & { k40: 40 } & { k41: 41 } & { k42: 42 } & { k43: 43 } & { k44: 44 } & { k45: 45 } & { k46: 46 } & { k47: 47 } & { k48: 48 } & { k49: 49 } & { k50: 50 } & { k51: 51 } & { k52: 52 } & { k53: 53 } & { k54: 54 } & { k55: 55 } & { k56: 56 } & { k57: 57 } & { k58: 58 } & { k59: 59 } & { k60: 60 } & { k61: 61 } & { k62: 62 } & { k63: 63 } & { k64: 64 } & { k65: 65 } & { k66: 66 } & { k67: 67 } & { k68: 68 } & { k69: 69 } & { k70: 70 } & { k71: 71 } & { k72: 72 } & { k73: 73 } & { k74: 74 } & { k75: 75 } & { k76: 76 } & { k77: 77 } & { k78: 78 } & { k79: 79 } & { k80: 80 } & { k81: 81 } & { k82: 82 } & { k83: 83 } & { k84: 84 } & { k85: 85 } & { k86: 86 } & { k87: 87 } & { k88: 88 } & { k89: 89 } & { k90: 90 } & { k91: 91 } & { k92: 92 } & { k93: 93 } & { k94: 94 } & { k95: 95 } & { k96: 96 } & { k97: 97 } & { k98: 98 } & { k99: 99 } & { k100: 100 } & { k101: 101 } & { k102: 102 } & { k103: 103 } & { k104: 104 } & { k105: 105 } & { k106: 106 } & { k107: 107 } & { k108: 108 } & { k109: 109 } & { k110: 110 } & { k111: 111 } & { k112: 112 } & { k113: 113 } & { k114: 114 } & { k115: 115 } & { k116: 116 } & { k117: 117 } & { k118: 118 } & { k119: 119 } & { k120: 120 } & { k121: 121 } & { k122: 122 } & { k123: 123 } & { k124: 124 } & { k125: 125 } & { k126: 126 } & { k127: 127 } & { k128: 128 } & { k129: 129 } & { k130: 130 } & { k131: 131 } & { k132: 132 } & { k133: 133 } & { k134: 134 } & { k135: 135 } & { k136: 136 } & { k137: 137 } & { k138: 138 } & { k139: 139 } & { k140: 140 } & { k141: 141 } & { k142: 142 } & { k143: 143 } & { k144: 144 } & { k145: 145 } & { k146: 146 } & { k147: 147 } & { k148: 148 } & { k149: 149 } & { k150: 150 } & { k151: 151 } & { k152: 152 } & { k153: 153 } & { k154: 154 } & { k155: 155 } & { k156: 156 } & { k157: 157 } & { k158: 158 } & { k159: 159 } & { k160: 160 } & { k161: 161 } & { k162: 162 } & { k163: 163 } & { k164: 164 } & { k165: 165 } & { k166: 166 } & { k167: 167 } & { k168: 168 } & { k169: 169 } & { k170: 170 } & { k171: 171 } & { k172: 172 } & { k173: 173 } & { k174: 174 } & { k175: 175 } & { k176: 176 } & { k177: 177 } & { k178: 178 } & { k179: 179 } & { k180: 180 } & { k181: 181 } & { k182: 182 } & { k183: 183 } & { k184: 184 } & { k185: 185 } & { k186: 186 } & { k187: 187 } & { k188: 188 } & { k189: 189 } & { k190: 190 } & { k191: 191 } & { k192: 192 } & { k193: 193 } & { k194: 194 } & { k195: 195 } & { k196: 196 } & { k197: 197 } & { k198: 198 } & { k199: 199 } & { k200: 200 } & { k201: 201 } & { k202: 202 } & { k203: 203 } & { k204: 204 } & { k205: 205 } & { k206: 206 } & { k207: 207 } & { k208: 208 } & { k209: 209 } & { k210: 210 } & { k211: 211 } & { k212: 212 } & { k213: 213 } & { k214: 214 } & { k215: 215 } & { k216: 216 } & { k217: 217 } & { k218: 218 } & { k219: 219 } & { k220: 220 } & { k221: 221 } & { k222: 222 } & { k223: 223 } & { k224: 224 } & { k225: 225 } & { k226: 226 } & { k227: 227 } & { k228: 228 } & { k229: 229 } & { k230: 230 } & { k231: 231 } & { k232: 232 } & { k233: 233 } & { k234: 234 } & { k235: 235 } & { k236: 236 } & { k237: 237 } & { k238: 238 } & { k239: 239 } & { k240: 240 } & { k241: 241 } & { k242: 242 } & { k243: 243 } & { k244: 244 } & { k245: 245 } & { k246: 246 } & { k247: 247 } & { k248: 248 } & { k249: 249 } & { k250: 250 } & { k251: 251 } & { k252: 252 } & { k253: 253 } & { k254: 254 } & { k255: 255 } & { k256: 256 } & { k257: 257 } & { k258: 258 } & { k259: 259 } & { k260: 260 } & { k261: 261 } & { k262: 262 } & { k263: 263 } & { k264: 264 } & { k265: 265 } & { k266: 266 } & { k267: 267 } & { k268: 268 } & { k269: 269 } & { k270: 270 } & { k271: 271 } & { k272: 272 } & { k273: 273 } & { k274: 274 } & { k275: 275 } & { k276: 276 } & { k277: 277 } & { k278: 278 } & { k279: 279 } & { k280: 280 } & { k281: 281 } & { k282: 282 } & { k283: 283 } & { k284: 284 } & { k285: 285 } & { k286: 286 } & { k287: 287 } & { k288: 288 } & { k289: 289 } & { k290: 290 } & { k291: 291 } & { k292: 292 } & { k293: 293 } & { k294: 294 } & { k295: 295 } & { k296: 296 } & { k297: 297 } & { k298: 298 } & { k299: 299 } & { k300: 300 } & { k301: 301 } & { k302: 302 } & { k303: 303 } & { k304: 304 } & { k305: 305 } & { k306: 306 } & { k307: 307 } & { k308: 308 } & { k309: 309 } & { k310: 310 } & { k311: 311 } & { k312: 312 } & { k313: 313 } & { k314: 314 } & { k315: 315 } & { k316: 316 } & { k317: 317 } & { k318: 318 } & { k319: 319 } & { k320: 320 } & { k321: 321 } & { k322: 322 } & { k323: 323 } & { k324: 324 } & { k325: 325 } & { k326: 326 } & { k327: 327 } & { k328: 328 } & { k329: 329 } & { k330: 330 } & { k331: 331 } & { k332: 332 } & { k333: 333 } & { k334: 334 } & { k335: 335 } & { k336: 336 } & { k337: 337 } & { k338: 338 } & { k339: 339 } & { k340: 340 } & { k341: 341 } & { k342: 342 } & { k343: 343 } & { k344: 344 } & { k345: 345 } & { k346: 346 } & { k347: 347 } & { k348: 348 } & { k349: 349 } & { k350: 350 } & { k351: 351 } & { k352: 352 } & { k353: 353 } & { k354: 354 } & { k355: 355 } & { k356: 356 } & { k357: 357 } & { k358: 358 } & { k359: 359 } & { k360: 360 } & { k361: 361 } & { k362: 362 } & { k363: 363 } & { k364: 364 } & { k365: 365 } & { k366: 366 } & { k367: 367 } & { k368: 368 } & { k369: 369 } & { k370: 370 } & { k371: 371 } & { k372: 372 } & { k373: 373 } & { k374: 374 } & { k375: 375 } & { k376: 376 } & { k377: 377 } & { k378: 378 } & { k379: 379 } & { k380: 380 } & { k381: 381 } & { k382: 382 } & { k383: 383 } & { k384: 384 } & { k385: 385 } & { k386: 386 } & { k387: 387 } & { k388: 388 } & { k389: 389 } & { k390: 390 } & { k391: 391 } & { k392: 392 } & { k393: 393 } & { k394: 394 } & { k395: 395 } & { k396: 396 } & { k397: 397 } & { k398: 398 } & { k399: 399 } & { k400: 400 } & { k401: 401 } & { k402: 402 } & { k403: 403 } & { k404: 404 } & { k405: 405 } & { k406: 406 } & { k407: 407 } & { k408: 408 } & { k409: 409 } & { k410: 410 } & { k411: 411 } & { k412: 412 } & { k413: 413 } & { k414: 414 } & { k415: 415 } & { k416: 416 } & { k417: 417 } & { k418: 418 } & { k419: 419 } & { k420: 420 } & { k421: 421 } & { k422: 422 } & { k423: 423 } & { k424: 424 } & { k425: 425 } & { k426: 426 } & { k427: 427 } & { k428: 428 } & { k429: 429 } & { k430: 430 } & { k431: 431 } & { k432: 432 } & { k433: 433 } & { k434: 434 } & { k435: 435 } & { k436: 436 } & { k437: 437 } & { k438: 438 } & { k439: 439 } & { k440: 440 } & { k441: 441 } & { k442: 442 } & { k443: 443 } & { k444: 444 } & { k445: 445 } & { k446: 446 } & { k447: 447 } & { k448: 448 } & { k449: 449 } & { k450: 450 } & { k451: 451 } & { k452: 452 } & { k453: 453 } & { k454: 454 } & { k455: 455 } & { k456: 456 } & { k457: 457 } & { k458: 458 } & { k459: 459 } & { k460: 460 } & { k461: 461 } & { k462: 462 } & { k463: 463 } & { k464: 464 } & { k465: 465 } & { k466: 466 } & { k467: 467 } & { k468: 468 } & { k469: 469 } & { k470: 470 } & { k471: 471 } & { k472: 472 } & { k473: 473 } & { k474: 474 } & { k475: 475 } & { k476: 476 } & { k477: 477 } & { k478: 478 } & { k479: 479 } & { k480: 480 } & { k481: 481 } & { k482: 482 } & { k483: 483 } & { k484: 484 } & { k485: 485 } & { k486: 486 } & { k487: 487 } & { k488: 488 } & { k489: 489 } & { k490: 490 } & { k491: 491 } & { k492: 492 } & { k493: 493 } & { k494: 494 } & { k495: 495 } & { k496: 496 } & { k497: 497 } & { k498: 498 } & { k499: 499 } & { k500: 500 } & { k501: 501 };
  nestedAllOf: { inside: { k1: 1 } & { k2: 2 } & { k3: 3 } & { k4: 4 } & { k5: 5 } & { k6: 6 } & { k7: 7 } & { k8: 8 } & { k9: 9 } & { k10: 10 } & { k11: 11 } & { k12: 12 } & { k13: 13 } & { k14: 14 } & { k15: 15 } & { k16: 16 } & { k17: 17 } & { k18: 18 } & { k19: 19 } & { k20: 20 } & { k21: 21 } & { k22: 22 } & { k23: 23 } & { k24: 24 } & { k25: 25 } & { k26: 26 } & { k27: 27 } & { k28: 28 } & { k29: 29 } & { k30: 30 } & { k31: 31 } & { k32: 32 } & { k33: 33 } & { k34: 34 } & { k35: 35 } & { k36: 36 } & { k37: 37 } & { k38: 38 } & { k39: 39 } & { k40: 40 } & { k41: 41 } & { k42: 42 } & { k43: 43 } & { k44: 44 } & { k45: 45 } & { k46: 46 } & { k47: 47 } & { k48: 48 } & { k49: 49 } & { k50: 50 } & { k51: 51 } & { k52: 52 } & { k53: 53 } & { k54: 54 } & { k55: 55 } & { k56: 56 } & { k57: 57 } & { k58: 58 } & { k59: 59 } & { k60: 60 } & { k61: 61 } & { k62: 62 } & { k63: 63 } & { k64: 64 } & { k65: 65 } & { k66: 66 } & { k67: 67 } & { k68: 68 } & { k69: 69 } & { k70: 70 } & { k71: 71 } & { k72: 72 } & { k73: 73 } & { k74: 74 } & { k75: 75 } & { k76: 76 } & { k77: 77 } & { k78: 78 } & { k79: 79 } & { k80: 80 } & { k81: 81 } & { k82: 82 } & { k83: 83 } & { k84: 84 } & { k85: 85 } & { k86: 86 } & { k87: 87 } & { k88: 88 } & { k89: 89 } & { k90: 90 } & { k91: 91 } & { k92: 92 } & { k93: 93 } & { k94: 94 } & { k95: 95 } & { k96: 96 } & { k97: 97 } & { k98: 98 } & { k99: 99 } & { k100: 100 } & { k101: 101 } & { k102: 102 } & { k103: 103 } & { k104: 104 } & { k105: 105 } & { k106: 106 } & { k107: 107 } & { k108: 108 } & { k109: 109 } & { k110: 110 } & { k111: 111 } & { k112: 112 } & { k113: 113 } & { k114: 114 } & { k115: 115 } & { k116: 116 } & { k117: 117 } & { k118: 118 } & { k119: 119 } & { k120: 120 } & { k121: 121 } };
  longTuple: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254, 255, 256, 257, 258, 259, 260, 261, 262, 263, 264, 265, 266, 267, 268, 269, 270, 271, 272, 273, 274, 275, 276, 277, 278, 279, 280, 281, 282, 283, 284, 285, 286, 287, 288, 289, 290, 291, 292, 293, 294, 295, 296, 297, 298, 299, 300, 301, 302, 303, 304, 305, 306, 307, 308, 309, 310, 311, 312, 313, 314, 315, 316, 317, 318, 319, 320, 321, 322, 323, 324, 325, 326, 327, 328, 329, 330, 331, 332, 333, 334, 335, 336, 337, 338, 339, 340, 341, 342, 343, 344, 345, 346, 347, 348, 349, 350, 351, 352, 353, 354, 355, 356, 357, 358, 359, 360, 361, 362, 363, 364, 365, 366, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378, 379, 380, 381, 382, 383, 384, 385, 386, 387, 388, 389, 390, 391, 392, 393, 394, 395, 396, 397, 398, 399, 400, 401, 402, 403, 404, 405, 406, 407, 408, 409, 410, 411, 412, 413, 414, 415, 416, 417, 418, 419, 420, 421, 422, 423, 424, 425, 426, 427, 428, 429, 430, 431, 432, 433, 434, 435, 436, 437, 438, 439, 440, 441, 442, 443, 444, 445, 446, 447, 448, 449, 450, 451, 452, 453, 454, 455, 456, 457, 458, 459, 460, 461, 462, 463, 464, 465, 466, 467, 468, 469, 470, 471, 472, 473, 474, 475, 476, 477, 478, 479, 480, 481, 482, 483, 484, 485, 486, 487, 488, 489, 490, 491, 492, 493, 494, 495, 496, 497, 498, 499, 500];
  nestedTuple: { inside: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120] };
  withinLimits: { inside: `${Digit}${Digit}` };
}) { return null; }

/** Depth 5 cuts each kind of type that has parts: the type is the value of `e` below four objects. */
export function DeepKinds(props: {
  union: { a: { b: { c: { d: { e: 'x' | 'y' } } } } };
  intersection: { a: { b: { c: { d: { e: { p: 1 } & { q: 2 } } } } } };
  array: { a: { b: { c: { d: { e: string[] } } } } };
  tuple: { a: { b: { c: { d: { e: [1, 2] } } } } };
  template: { a: { b: { c: { d: { e: `${number}px` } } } } };
  scalar: { a: { b: { c: { d: { e: string } } } } };
}) { return null; }

/** Open type parameters, keyof, indexed access and conditional types that cannot be resolved. */
export function Open<T extends { k: string }>(props: { item: T; key: keyof T; value: T['k']; cond: T extends string ? 1 : 2; fn: <U>(u: U) => U }) { return null; }

/** Unions of objects, and an intersection of a union with an object. */
type Circle = { kind: 'circle'; r: number };
type Square = { kind: 'square'; side: number };
export function Unions(props: { shape: Circle | Square; both: (Circle | Square) & { id: string }; nullable?: Circle | null }) { return null; }

/** An unresolved import is the checker's error type. */
export function Lost(props: { gone: import('./missing').Gone }) { return null; }
