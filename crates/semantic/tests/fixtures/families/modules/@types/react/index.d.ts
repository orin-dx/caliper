// A stand-in for @types/react with the parts that make its DOM attribute types wide: an attribute interface of more than
// a hundred properties that others extend, `DetailedHTMLProps` over `ClassAttributes`, `ComponentProps` through
// `PropsWithoutRef`, and a `ReactNode` that holds `ReactPortal`, an `Iterable` of itself and a `Promise`. It sits at
// node_modules/@types/react/ in the test project.

export type ReactNode =
  | ReactElement
  | string
  | number
  | bigint
  | Iterable<ReactNode>
  | ReactPortal
  | boolean
  | null
  | undefined
  | Promise<AwaitedReactNode>;
export type AwaitedReactNode =
  | ReactElement
  | string
  | number
  | bigint
  | Iterable<ReactNode>
  | ReactPortal
  | boolean
  | null
  | undefined;

export interface ReactElement<P = any, T = any> {
  type: T;
  props: P;
  key: string | null;
}
export interface ReactPortal extends ReactElement {
  children: ReactNode;
}

export interface FunctionComponent<P = {}> {
  (props: P): ReactNode;
  displayName?: string;
}
export interface ComponentClass<P = {}> {
  new (props: P): { props: P; render(): ReactNode };
  displayName?: string;
}
export type ComponentType<P = {}> = ComponentClass<P> | FunctionComponent<P>;
export type ElementType<P = any, Tag extends keyof JSX.IntrinsicElements = keyof JSX.IntrinsicElements> =
  | { [K in Tag]: P extends JSX.IntrinsicElements[K] ? K : never }[Tag]
  | ComponentType<P>;

export interface SyntheticEvent<T = Element> {
  currentTarget: T;
}
export type MouseEventHandler<T> = (event: SyntheticEvent<T>) => void;

export type Ref<T> = { current: T | null } | null;
export interface ClassAttributes<T> {
  ref?: Ref<T>;
  key?: string;
}

export interface DOMAttributes<T> {
  children?: ReactNode;
  onClick?: MouseEventHandler<T>;
}
export interface HTMLAttributes<T> extends DOMAttributes<T> {
  attr001?: string; attr002?: string; attr003?: string; attr004?: string; attr005?: string; attr006?: string; attr007?: string; attr008?: string; attr009?: string; attr010?: string; attr011?: string; attr012?: string; attr013?: string; attr014?: string; attr015?: string; attr016?: string; attr017?: string; attr018?: string; attr019?: string; attr020?: string; attr021?: string; attr022?: string; attr023?: string; attr024?: string; attr025?: string; attr026?: string; attr027?: string; attr028?: string; attr029?: string; attr030?: string; attr031?: string; attr032?: string; attr033?: string; attr034?: string; attr035?: string; attr036?: string; attr037?: string; attr038?: string; attr039?: string; attr040?: string; attr041?: string; attr042?: string; attr043?: string; attr044?: string; attr045?: string; attr046?: string; attr047?: string; attr048?: string; attr049?: string; attr050?: string; attr051?: string; attr052?: string; attr053?: string; attr054?: string; attr055?: string; attr056?: string; attr057?: string; attr058?: string; attr059?: string; attr060?: string; attr061?: string; attr062?: string; attr063?: string; attr064?: string; attr065?: string; attr066?: string; attr067?: string; attr068?: string; attr069?: string; attr070?: string; attr071?: string; attr072?: string; attr073?: string; attr074?: string; attr075?: string; attr076?: string; attr077?: string; attr078?: string; attr079?: string; attr080?: string; attr081?: string; attr082?: string; attr083?: string; attr084?: string; attr085?: string; attr086?: string; attr087?: string; attr088?: string; attr089?: string; attr090?: string; attr091?: string; attr092?: string; attr093?: string; attr094?: string; attr095?: string; attr096?: string; attr097?: string; attr098?: string; attr099?: string; attr100?: string; attr101?: string; attr102?: string; attr103?: string; attr104?: string; attr105?: string; attr106?: string; attr107?: string; attr108?: string; attr109?: string; attr110?: string;
  color?: string;
  style?: { width?: number };
}
export interface AnchorHTMLAttributes<T> extends HTMLAttributes<T> {
  href?: string;
  target?: string;
}
export interface InputHTMLAttributes<T> extends HTMLAttributes<T> {
  value?: string;
  disabled?: boolean;
  type?: 'text' | 'number';
}
export interface ButtonHTMLAttributes<T> extends HTMLAttributes<T> {
  disabled?: boolean;
  type?: 'button' | 'submit';
}
export type DetailedHTMLProps<E extends HTMLAttributes<T>, T> = ClassAttributes<T> & E;

declare global {
  namespace JSX {
    interface IntrinsicElements {
      a: DetailedHTMLProps<AnchorHTMLAttributes<HTMLAnchorElement>, HTMLAnchorElement>;
      button: DetailedHTMLProps<ButtonHTMLAttributes<HTMLButtonElement>, HTMLButtonElement>;
      div: DetailedHTMLProps<HTMLAttributes<HTMLDivElement>, HTMLDivElement>;
      input: DetailedHTMLProps<InputHTMLAttributes<HTMLInputElement>, HTMLInputElement>;
      tag001: {}; tag002: {}; tag003: {}; tag004: {}; tag005: {}; tag006: {}; tag007: {}; tag008: {}; tag009: {}; tag010: {}; tag011: {}; tag012: {}; tag013: {}; tag014: {}; tag015: {}; tag016: {}; tag017: {}; tag018: {}; tag019: {}; tag020: {}; tag021: {}; tag022: {}; tag023: {}; tag024: {}; tag025: {}; tag026: {}; tag027: {}; tag028: {}; tag029: {}; tag030: {}; tag031: {}; tag032: {}; tag033: {}; tag034: {}; tag035: {}; tag036: {}; tag037: {}; tag038: {}; tag039: {}; tag040: {}; tag041: {}; tag042: {}; tag043: {}; tag044: {}; tag045: {}; tag046: {}; tag047: {}; tag048: {}; tag049: {}; tag050: {}; tag051: {}; tag052: {}; tag053: {}; tag054: {}; tag055: {}; tag056: {}; tag057: {}; tag058: {}; tag059: {}; tag060: {}; tag061: {}; tag062: {}; tag063: {}; tag064: {}; tag065: {}; tag066: {}; tag067: {}; tag068: {}; tag069: {}; tag070: {}; tag071: {}; tag072: {}; tag073: {}; tag074: {}; tag075: {}; tag076: {}; tag077: {}; tag078: {}; tag079: {}; tag080: {}; tag081: {}; tag082: {}; tag083: {}; tag084: {}; tag085: {}; tag086: {}; tag087: {}; tag088: {}; tag089: {}; tag090: {}; tag091: {}; tag092: {}; tag093: {}; tag094: {}; tag095: {}; tag096: {}; tag097: {}; tag098: {}; tag099: {}; tag100: {}; tag101: {}; tag102: {}; tag103: {}; tag104: {}; tag105: {}; tag106: {}; tag107: {}; tag108: {}; tag109: {}; tag110: {}; tag111: {}; tag112: {}; tag113: {}; tag114: {}; tag115: {}; tag116: {}; tag117: {}; tag118: {}; tag119: {}; tag120: {};
    }
  }
}

export type ComponentProps<T extends keyof JSX.IntrinsicElements> = JSX.IntrinsicElements[T];
export type PropsWithoutRef<P> = P extends any ? ('ref' extends keyof P ? Omit<P, 'ref'> : P) : P;
export type ComponentPropsWithoutRef<T extends keyof JSX.IntrinsicElements> = PropsWithoutRef<ComponentProps<T>>;
