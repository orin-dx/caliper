// A small stand-in for @types/react: the declarations the cases use, shaped as the real ones are (a recursive ReactNode
// union, an element type built from a conditional type, event handlers behind a type alias), so that the checker
// answers about them the way it does about the real package. It sits at node_modules/@types/react/ in the test project.

export type ReactNode = ReactElement | string | number | bigint | Iterable<ReactNode> | boolean | null | undefined;

export interface ReactElement<P = any, T = any> {
  type: T;
  props: P;
  key: string | null;
}

export type ComponentType<P = {}> = (props: P) => ReactNode;

export interface HTMLAttributes<T> {
  className?: string;
  id?: string;
  title?: string;
  onClick?: MouseEventHandler<T>;
  children?: ReactNode;
}

export interface ButtonHTMLAttributes<T> extends HTMLAttributes<T> {
  disabled?: boolean;
  type?: 'button' | 'submit' | 'reset';
}

export interface AnchorHTMLAttributes<T> extends HTMLAttributes<T> {
  href?: string;
}

interface IntrinsicProps {
  div: HTMLAttributes<HTMLDivElement>;
  a: AnchorHTMLAttributes<HTMLAnchorElement>;
  button: ButtonHTMLAttributes<HTMLButtonElement>;
}

export type ElementType<P = any> = keyof IntrinsicProps | ComponentType<P>;

export type ComponentPropsWithoutRef<E> = E extends keyof IntrinsicProps
  ? IntrinsicProps[E]
  : E extends ComponentType<infer P>
    ? P
    : never;

export interface SyntheticEvent<T = Element> {
  currentTarget: T;
  type: string;
}
export interface ChangeEvent<T = Element> extends SyntheticEvent<T> {
  target: T;
}
export interface MouseEvent<T = Element> extends SyntheticEvent<T> {
  clientX: number;
}
export type MouseEventHandler<T> = (event: MouseEvent<T>) => void;

export interface RefObject<T> {
  current: T | null;
}
export type RefCallback<T> = (instance: T | null) => void;
export type Ref<T> = RefCallback<T> | RefObject<T> | null;
export interface RefAttributes<T> {
  ref?: Ref<T>;
}

export interface CSSProperties {
  color?: string;
  width?: number | string;
}

export interface FunctionComponent<P = {}> {
  (props: P): ReactNode;
  displayName?: string;
}
export type FC<P = {}> = FunctionComponent<P>;

export interface ForwardRefExoticComponent<P> {
  (props: P): ReactNode;
  displayName?: string;
}
export function forwardRef<T, P = {}>(
  render: (props: P, ref: Ref<T>) => ReactNode,
): ForwardRefExoticComponent<P & RefAttributes<T>>;

export class Component<P = {}> {
  constructor(props: P);
  props: P;
  render(): ReactNode;
}

declare global {
  namespace JSX {
    interface Element extends ReactElement<any, any> {}
    interface IntrinsicElements {
      [name: string]: any;
    }
  }
}
