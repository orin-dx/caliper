//! Pins the on-disk shape of `SourceData`. The cache is positional MessagePack (ADR 0003), so reordering a field or
//! adding a variant silently corrupts old entries unless `CACHE_SCHEMA_VERSION` moves. A change here that is not
//! accompanied by a version bump is the bug this test exists to catch.

use camino::Utf8Path;

use super::CACHE_SCHEMA_VERSION;
use crate::extractor::parse_file;
use crate::types::collected::SourceData;

const FIXTURE: &str = r#"
import React, { forwardRef } from "react";
import type { Base } from "./base";

/** A size scale. */
export const SIZES = ["sm", "md", "lg"] as const;

export enum Tone {
  Quiet = "quiet",
  Loud = "loud",
}

type Pick2<T, K extends keyof T> = { [P in K]: T[P] };

/** Button props. */
export interface ButtonProps<TValue = string> extends Base {
  /** Visual tone. @default "quiet" */
  tone?: Tone;
  size: (typeof SIZES)[number];
  value?: TValue;
  onPress?: (event: MouseEvent) => void;
}

export type Alias = Pick2<ButtonProps, "tone"> & { extra?: number };

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(props, ref) {
  return <button ref={ref} />;
});

export function Card(props: Alias) {
  return <div />;
}

const Opaque = makeThing();
"#;

fn fixture() -> SourceData {
    parse_file(Utf8Path::new("/golden/button.tsx"), FIXTURE)
}

#[test]
fn fixture_populates_every_source_data_field() {
    let SourceData {
        interfaces,
        type_aliases,
        type_alias_params,
        interface_type_params,
        enums,
        const_arrays,
        component_mappings,
        imports,
        exports,
        diagnostics,
    } = fixture();
    let fields = [
        ("interfaces", interfaces.is_empty()),
        ("type_aliases", type_aliases.is_empty()),
        ("type_alias_params", type_alias_params.is_empty()),
        ("interface_type_params", interface_type_params.is_empty()),
        ("enums", enums.is_empty()),
        ("const_arrays", const_arrays.is_empty()),
        ("component_mappings", component_mappings.is_empty()),
        ("imports", imports.is_empty()),
        ("exports", exports.is_empty()),
        ("diagnostics", diagnostics.is_empty()),
    ];
    let empty: Vec<&str> = fields.iter().filter(|(_, is_empty)| *is_empty).map(|(name, _)| *name).collect();
    assert_eq!(empty, Vec::<&str>::new(), "extend FIXTURE so the golden bytes cover these fields");
}

#[test]
fn source_data_bytes_are_stable_for_the_current_schema_version() {
    let bytes = rmp_serde::to_vec(&fixture()).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    insta::assert_snapshot!(format!("schema {CACHE_SCHEMA_VERSION}\n{hex}"));
}

#[test]
fn source_data_round_trips_through_msgpack() {
    let original = fixture();
    let decoded: SourceData = rmp_serde::from_slice(&rmp_serde::to_vec(&original).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), serde_json::to_value(&original).unwrap());
}
