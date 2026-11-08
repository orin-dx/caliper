export type Tone = 'calm' | 'loud';

export interface DepBaseProps {
  /** Tone from a dependency. */
  tone?: Tone;
  onTone?: (t: Tone, ev: { x: number }) => void;
}

/** An object type a package names: a reader gets the name, not its fields. */
export type DepStyle = { color: string; weight: number; nested: { deep: boolean } };

/** An interface a package names. */
export interface DepTheme {
  name: string;
  scale: number[];
}

/** A literal union written through another alias, both in the package. */
export type DepSize = Tone | 'tiny';
