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

/** The caps: a type deeper than the depth limit, an object wider than the field limit, a union longer than the member limit. */
type Deep = { a: { b: { c: { d: { e: { f: { g: string } } } } } } };
interface Wide {
  f01: 1; f02: 2; f03: 3; f04: 4; f05: 5; f06: 6; f07: 7; f08: 8; f09: 9; f10: 10;
  f11: 11; f12: 12; f13: 13; f14: 14; f15: 15; f16: 16; f17: 17; f18: 18; f19: 19; f20: 20;
  f21: 21; f22: 22; f23: 23; f24: 24; f25: 25; f26: 26; f27: 27; f28: 28; f29: 29; f30: 30;
  f31: 31; f32: 32; f33: 33; f34: 34; f35: 35; f36: 36; f37: 37; f38: 38; f39: 39; f40: 40;
  f41: 41; f42: 42;
}
export function Caps(props: { deep: Deep; wide: { inside: Wide }; many: `${0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9}${0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9}${0 | 1 | 2 | 3 | 4}` }) { return null; }

/** Open type parameters, keyof, indexed access and conditional types that cannot be resolved. */
export function Open<T extends { k: string }>(props: { item: T; key: keyof T; value: T['k']; cond: T extends string ? 1 : 2; fn: <U>(u: U) => U }) { return null; }

/** Unions of objects, and an intersection of a union with an object. */
type Circle = { kind: 'circle'; r: number };
type Square = { kind: 'square'; side: number };
export function Unions(props: { shape: Circle | Square; both: (Circle | Square) & { id: string }; nullable?: Circle | null }) { return null; }

/** An unresolved import is the checker's error type. */
export function Lost(props: { gone: import('./missing').Gone }) { return null; }
