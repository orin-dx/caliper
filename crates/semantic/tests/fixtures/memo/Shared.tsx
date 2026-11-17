// A type that is shared, and that many different types enclose: converting it once per enclosing type is the cost
// `tests/memo.rs` holds down. Nothing here is recursive, so no enclosing type changes what `Shared` is.

interface Leaf { id: string; label: string; count: number; on: boolean }
interface Branch { left: Leaf; right: Leaf; tags: string[]; extra?: Leaf }
export interface Shared {
  name: string;
  size: 'sm' | 'md' | 'lg';
  first: Branch;
  second: Branch;
  third: Branch;
  list: Leaf[];
  pair: [Leaf, Branch];
}

interface W1 { shared: Shared; own1: number }
interface W2 { shared: Shared; own2: number }
interface W3 { shared: Shared; own3: number }
interface W4 { shared: Shared; own4: number }
interface W5 { shared: Shared; own5: number }
interface W6 { shared: Shared; own6: number }
interface W7 { shared: Shared; own7: number }
interface W8 { shared: Shared; own8: number }
interface W9 { shared: Shared; own9: number }
interface W10 { shared: Shared; own10: number }
interface W11 { shared: Shared; own11: number }
interface W12 { shared: Shared; own12: number }
interface W13 { shared: Shared; own13: number }
interface W14 { shared: Shared; own14: number }
interface W15 { shared: Shared; own15: number }
interface W16 { shared: Shared; own16: number }

/** The same interface under many enclosing instantiations. */
interface Box<T> { value: T; shared: Shared }
type Deep = Box<Box<Box<Box<string>>>>;

/** The same type under sixteen different enclosing interfaces. */
export function Many(props: {
  w1: W1;
  w2: W2;
  w3: W3;
  w4: W4;
  w5: W5;
  w6: W6;
  w7: W7;
  w8: W8;
  w9: W9;
  w10: W10;
  w11: W11;
  w12: W12;
  w13: W13;
  w14: W14;
  w15: W15;
  w16: W16;
  box1: Box<string>;
  box2: Box<number>;
  box3: Box<Box<string>>;
  deep: Deep;
}) { return null; }

type U1 = { kind: 'u1'; shared: Shared; payload1: Leaf };
type U2 = { kind: 'u2'; shared: Shared; payload2: Leaf };
type U3 = { kind: 'u3'; shared: Shared; payload3: Leaf };
type U4 = { kind: 'u4'; shared: Shared; payload4: Leaf };
type U5 = { kind: 'u5'; shared: Shared; payload5: Leaf };
type U6 = { kind: 'u6'; shared: Shared; payload6: Leaf };
type U7 = { kind: 'u7'; shared: Shared; payload7: Leaf };
type U8 = { kind: 'u8'; shared: Shared; payload8: Leaf };
type U9 = { kind: 'u9'; shared: Shared; payload9: Leaf };
type U10 = { kind: 'u10'; shared: Shared; payload10: Leaf };
type U11 = { kind: 'u11'; shared: Shared; payload11: Leaf };
type U12 = { kind: 'u12'; shared: Shared; payload12: Leaf };
type Wide = U1 | U2 | U3 | U4 | U5 | U6 | U7 | U8 | U9 | U10 | U11 | U12;

/** A wide union of aliases that all mention the one big object. */
export function Unions(props: { wide: Wide; wides: Wide[]; held: { inner: Wide } }) { return null; }
