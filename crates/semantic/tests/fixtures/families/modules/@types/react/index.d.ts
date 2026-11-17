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
      abbr: {}; address: {}; animate: {}; animateMotion: {}; animateTransform: {}; area: {}; article: {}; aside: {}; audio: {}; b: {}; base: {}; bdi: {}; bdo: {}; big: {}; blockquote: {}; body: {}; br: {}; canvas: {}; caption: {}; center: {}; circle: {}; cite: {}; clipPath: {}; code: {}; col: {}; colgroup: {}; data: {}; datalist: {}; dd: {}; defs: {}; del: {}; desc: {}; details: {}; dfn: {}; dialog: {}; dl: {}; dt: {}; ellipse: {}; em: {}; embed: {}; feBlend: {}; feColorMatrix: {}; feComponentTransfer: {}; feComposite: {}; feConvolveMatrix: {}; feDiffuseLighting: {}; feDisplacementMap: {}; feDistantLight: {}; feDropShadow: {}; feFlood: {}; feFuncA: {}; feFuncB: {}; feFuncG: {}; feFuncR: {}; feGaussianBlur: {}; feImage: {}; feMerge: {}; feMergeNode: {}; feMorphology: {}; feOffset: {}; fePointLight: {}; feSpecularLighting: {}; feSpotLight: {}; feTile: {}; feTurbulence: {}; fieldset: {}; figcaption: {}; figure: {}; filter: {}; footer: {}; foreignObject: {}; form: {}; g: {}; h1: {}; h2: {}; h3: {}; h4: {}; h5: {}; h6: {}; head: {}; header: {}; hgroup: {}; hr: {}; html: {}; i: {}; iframe: {}; image: {}; img: {}; ins: {}; kbd: {}; keygen: {}; label: {}; legend: {}; li: {}; line: {}; linearGradient: {}; link: {}; main: {}; map: {}; mark: {}; marker: {}; mask: {}; menu: {}; menuitem: {}; meta: {}; metadata: {}; meter: {}; mpath: {}; nav: {}; noindex: {}; noscript: {}; object: {}; ol: {}; optgroup: {}; option: {}; output: {}; p: {}; param: {}; path: {}; pattern: {}; picture: {}; polygon: {}; polyline: {}; pre: {}; progress: {}; q: {}; radialGradient: {}; rect: {}; rp: {}; rt: {}; ruby: {}; s: {}; samp: {}; script: {}; search: {}; section: {}; select: {}; set: {}; slot: {}; small: {}; source: {}; span: {}; stop: {}; strong: {}; style: {}; sub: {}; summary: {}; sup: {}; svg: {}; switch: {}; symbol: {}; table: {}; tbody: {}; td: {}; template: {}; text: {}; textPath: {}; textarea: {}; tfoot: {}; th: {}; thead: {}; time: {}; title: {}; tr: {}; track: {}; tspan: {}; u: {}; ul: {}; use: {}; var: {}; video: {}; view: {}; wbr: {}; webview: {};
    }
  }
}

export type ComponentProps<T extends keyof JSX.IntrinsicElements> = JSX.IntrinsicElements[T];
export type PropsWithoutRef<P> = P extends any ? ('ref' extends keyof P ? Omit<P, 'ref'> : P) : P;
export type ComponentPropsWithoutRef<T extends keyof JSX.IntrinsicElements> = PropsWithoutRef<ComponentProps<T>>;
