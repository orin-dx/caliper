import * as React from 'react';
import type { DepBaseProps } from 'fake-dep';

/** 1 plain interface */
export interface PlainProps {
  /** The label. */
  label: string;
  /** Count. @default 3 */
  count?: number;
  disabled?: boolean;
  size?: 'sm' | 'md' | 'lg';
  readonly id?: string;
  children?: React.ReactNode;
}
export function Plain({ label, count = 3 }: PlainProps) { return <div>{label}{count}</div>; }

/** 2 forwardRef */
export interface FwdProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'solid' | 'ghost';
}
export const Fwd = React.forwardRef<HTMLButtonElement, FwdProps>((props, ref) => <button ref={ref} {...props} />);

/** 3 polymorphic generic */
type PolyProps<E extends React.ElementType> = { as?: E; tone?: 'a' | 'b' } & Omit<React.ComponentPropsWithoutRef<E>, 'as' | 'tone'>;
export function Poly<E extends React.ElementType = 'div'>(props: PolyProps<E>) { return null; }

/** 4 mapped / Omit / Partial */
interface Base { a: string; b: number; c?: boolean; d: () => void }
type Mapped = Partial<Omit<Base, 'd'>> & { [K in 'x' | 'y']: number };
export function Mapper(props: Mapped & { extra: Pick<Base, 'a'> }) { return null; }

/** 5 conditional */
type Cond<T> = T extends string ? { text: T } : { num: T };
export function CondC<T extends string | number>(props: Cond<T> & { k: T extends string ? 'str' : 'other' }) { return null; }

/** 6 literal union with string & {} */
type Color = 'red' | 'green' | (string & {});
export function Colored(props: { color?: Color; level?: 1 | 2 | 3; flag: true | false }) { return null; }

/** 7 callback */
export function Cb(props: { onChange: (value: string, ev?: React.ChangeEvent<HTMLInputElement>, ...rest: number[]) => void; render?: (x: { a: number }) => React.ReactNode }) { return null; }

/** 8 tuple */
export function Tup(props: { pos: [number, number?]; named: [x: number, y: string]; rest: [string, ...number[]]; ro: readonly [1, 2]; arr: readonly string[] }) { return null; }

/** 9 index signature */
export function Idx(props: { [key: string]: number | string; fixed: string; [k: `data-${string}`]: string }) { return null; }

/** 10 overloaded callable */
export interface OverProps { a: string }
export interface OverProps2 { b: number }
export declare function Over(props: OverProps): JSX.Element;
export declare function Over(props: OverProps2, ctx: unknown): JSX.Element;

/** 11 dependency declared prop */
export interface DepProps extends DepBaseProps { own: string }
export function Dep(props: DepProps) { return null; }

/** 12 class component with defaultProps */
export class Klass extends React.Component<{ title: string; n?: number }> {
  static defaultProps = { n: 1 };
  render() { return null; }
}

/** 13 recursive + unique symbol + enum + bigint + template */
export enum Dir { Up = 'UP', Down = 'DOWN' }
declare const sym: unique symbol;
export interface Tree { value: string; children?: Tree[]; parent?: Tree }
export function Misc(props: { tree: Tree; dir: Dir; big: bigint; tpl: `row-${number}`; s: typeof sym; fn: { (a: string): void; (a: number): void }; getter?: { get x(): number } }) { return null; }

/** 14 React.FC + lazy + memo */
export const Fc: React.FC<{ a?: string }> = () => null;
