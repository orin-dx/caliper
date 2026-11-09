import * as React from 'react';
import type { DepWide, DepSmall } from 'fake-dep';

// One component per way a wide type is kept: what the project declares is listed whole however wide, and what comes
// from a dependency is named. The widths are the checker's: 150 own fields, 150 from a package, React's DOM attributes
// (more than a hundred).

/** The project's own interface of 150 fields: never cut. */
export interface OwnWide {
  o001?: 1; o002?: 2; o003?: 3; o004?: 4; o005?: 5; o006?: 6; o007?: 7; o008?: 8; o009?: 9; o010?: 10; o011?: 11; o012?: 12; o013?: 13; o014?: 14; o015?: 15; o016?: 16; o017?: 17; o018?: 18; o019?: 19; o020?: 20; o021?: 21; o022?: 22; o023?: 23; o024?: 24; o025?: 25; o026?: 26; o027?: 27; o028?: 28; o029?: 29; o030?: 30; o031?: 31; o032?: 32; o033?: 33; o034?: 34; o035?: 35; o036?: 36; o037?: 37; o038?: 38; o039?: 39; o040?: 40; o041?: 41; o042?: 42; o043?: 43; o044?: 44; o045?: 45; o046?: 46; o047?: 47; o048?: 48; o049?: 49; o050?: 50; o051?: 51; o052?: 52; o053?: 53; o054?: 54; o055?: 55; o056?: 56; o057?: 57; o058?: 58; o059?: 59; o060?: 60; o061?: 61; o062?: 62; o063?: 63; o064?: 64; o065?: 65; o066?: 66; o067?: 67; o068?: 68; o069?: 69; o070?: 70; o071?: 71; o072?: 72; o073?: 73; o074?: 74; o075?: 75; o076?: 76; o077?: 77; o078?: 78; o079?: 79; o080?: 80; o081?: 81; o082?: 82; o083?: 83; o084?: 84; o085?: 85; o086?: 86; o087?: 87; o088?: 88; o089?: 89; o090?: 90; o091?: 91; o092?: 92; o093?: 93; o094?: 94; o095?: 95; o096?: 96; o097?: 97; o098?: 98; o099?: 99; o100?: 100; o101?: 101; o102?: 102; o103?: 103; o104?: 104; o105?: 105; o106?: 106; o107?: 107; o108?: 108; o109?: 109; o110?: 110; o111?: 111; o112?: 112; o113?: 113; o114?: 114; o115?: 115; o116?: 116; o117?: 117; o118?: 118; o119?: 119; o120?: 120; o121?: 121; o122?: 122; o123?: 123; o124?: 124; o125?: 125; o126?: 126; o127?: 127; o128?: 128; o129?: 129; o130?: 130; o131?: 131; o132?: 132; o133?: 133; o134?: 134; o135?: 135; o136?: 136; o137?: 137; o138?: 138; o139?: 139; o140?: 140; o141?: 141; o142?: 142; o143?: 143; o144?: 144; o145?: 145; o146?: 146; o147?: 147; o148?: 148; o149?: 149; o150?: 150;
}
export function Own(props: { wide: OwnWide; nested: { inside: OwnWide } }) { return null; }

/** A dependency's 150 fields stand as the name its layer has, in a wrapper or under the project's own fields. */
interface ExtendsDep extends DepWide {
  own: string;
  second?: number;
}
interface ExtendsTwice extends ExtendsDep {
  third: boolean;
}
export function Dependency(props: {
  omitted: Omit<DepWide, 'f001'>;
  partial: Partial<DepWide>;
  extended: ExtendsDep;
  chain: ExtendsTwice;
  small: Omit<DepSmall, 'a'>;
}) { return null; }

/** A project interface that extends a wrapper over React's DOM attributes, and its own fields beside them. */
interface BoxProps extends Omit<React.HTMLAttributes<HTMLDivElement>, 'color'> {
  color?: 'red' | 'blue';
  as?: React.ElementType;
}
interface Boxed<T> extends Omit<React.HTMLAttributes<HTMLDivElement>, 'color'> {
  value: T;
}
export function Boxes(props: { box?: BoxProps; boxed?: Boxed<string> }) { return null; }

/** Wrappers over a React attribute family: kept as a name when wide, expanded when the result is small. */
export function Wrappers(props: {
  link?: Omit<React.ComponentPropsWithoutRef<'a'>, 'href'>;
  picked?: Pick<React.ComponentProps<'button'>, 'disabled' | 'type'>;
  partial?: Partial<React.HTMLAttributes<HTMLDivElement>>;
  twice?: Omit<Omit<React.HTMLAttributes<HTMLSpanElement>, 'color'>, 'style'>;
  detailed?: React.DetailedHTMLProps<React.InputHTMLAttributes<HTMLInputElement>, HTMLInputElement>;
  direct?: React.AnchorHTMLAttributes<HTMLAnchorElement>;
  required?: Required<React.AnchorHTMLAttributes<HTMLAnchorElement>>;
  unknownElement?: Omit<React.HTMLAttributes<HTMLTableCellElement>, 'color'>;
}) { return null; }

/**
 * `ElementType` below the top of a prop is the union of over a hundred tag names and the component types, written with
 * `undefined` by its optional property: wider than the budget, it is the terminal and not a cut.
 */
export function Polymorphic(props: { at: { as?: React.ElementType }; top?: React.ElementType; few: { as?: 'a' | 'b' | React.ComponentType } }) { return null; }

/** ReactNode inside a union arrives as its members: it is the terminal again, and what else is in the union stays. */
export function Terminals(props: {
  node?: React.ReactNode | ((open: boolean) => React.ReactNode);
  portal?: React.ReactPortal | string;
  mixed?: React.ReactNode | Promise<number> | { id: string };
}) { return null; }
