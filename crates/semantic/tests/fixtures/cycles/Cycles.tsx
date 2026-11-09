// A type reached inside a cycle and outside it must be the same answer in both places, in either order: `Node2` is
// `Node1`'s member and `Node1` is `Node2`'s, so each is a back-reference only where the other encloses it.

interface Node1 { next: Node2; id: number }
interface Node2 { back: Node1; tag: string }
interface Holder { held: Node2 }
interface Wrapper<T> { inner: T }

/** The type outside the cycle first, then inside it. */
export function OutsideFirst(props: {
  holder: Holder;
  first: Node1;
  second: Node2;
  wrapped: Wrapper<Node1>;
  both: { a: Node1; b: Node2 };
}) { return null; }

/** The same props the other way round, so the entry made inside the cycle is the one that is there first. */
export function InsideFirst(props: {
  both: { a: Node1; b: Node2 };
  wrapped: Wrapper<Node1>;
  second: Node2;
  first: Node1;
  holder: Holder;
}) { return null; }
