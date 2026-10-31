//! Pins the on-disk shape of `SourceData`. The cache is positional MessagePack (ADR 0003), so reordering a field or
//! adding a variant silently corrupts old entries unless `CACHE_SCHEMA_VERSION` moves. A shape change here that is not
//! accompanied by a version bump is the bug this test exists to catch; the source fingerprint covers the rest.

use camino::Utf8Path;

use super::CACHE_SCHEMA_VERSION;
use crate::extractor::parse_file;
use crate::types::collected::SourceData;

const FIXTURE: &str = r#"
import React, { forwardRef } from "react";
import type { Base } from "./base";

/** A size scale. */
export const SIZES = ["sm", "md", "lg"] as const;
export const ALL_SIZES = ["xs", ...SIZES] as const;

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
  tags?: readonly string[];
  slots: { readonly id: string; [key: string]: number };
}

export type Alias = Pick2<ButtonProps, "tone"> & { extra?: number };

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(props, ref) {
  return <button ref={ref} />;
});

export function Card(props: Alias) {
  return <div />;
}

const Opaque = makeThing();

export function make<P>(render: (props: P) => null) {
  return render as unknown as React.FC<P & { as?: string }>;
}

export const Panel = make<PanelProps>((props) => <div />);
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
        function_results,
        type_param_defaults,
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
        ("function_results", function_results.is_empty()),
        ("type_param_defaults", type_param_defaults.is_empty()),
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

#[test]
fn a_blob_from_the_previous_schema_version_is_rejected() {
    let dir = std::env::temp_dir().join(format!("caliper-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cache_dir = camino::Utf8PathBuf::from_path_buf(dir.join("cache")).unwrap();
    let source_path = camino::Utf8PathBuf::from_path_buf(dir.join("button.d.ts")).unwrap();
    std::fs::write(&source_path, FIXTURE).unwrap();

    let writer = super::DtsCache::load_from_disk(Some(&cache_dir));
    writer.insert(&source_path, FIXTURE, fixture());
    assert!(writer.save_to_disk().is_none());
    assert!(super::DtsCache::load_from_disk(Some(&cache_dir)).get(&source_path, FIXTURE).is_some());

    // The same entries under the header the previous schema version wrote.
    let header = super::CacheHeader { schema: CACHE_SCHEMA_VERSION - 1, ..super::CacheHeader::current() };
    let key = super::SerializableCacheKey { path: source_path.to_string(), content_hash: super::hash_content(FIXTURE) };
    let mut bytes = rmp_serde::to_vec(&header).unwrap();
    bytes.extend(rmp_serde::to_vec(&vec![(key, fixture())]).unwrap());
    std::fs::write(cache_dir.join(super::DATA_FILE), bytes).unwrap();
    let reloaded = super::DtsCache::load_from_disk(Some(&cache_dir));
    let loaded = reloaded.get(&source_path, FIXTURE).is_some();
    std::fs::remove_dir_all(&dir).unwrap();

    assert!(!loaded, "entries written under schema {} must not load", CACHE_SCHEMA_VERSION - 1);
}
