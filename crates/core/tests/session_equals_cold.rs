#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! After any sequence of edits, a watch session's output equals a cold extract of the same files. A small project (a
//! component whose props come from an imported type, a re-exported type, wrappers, a `.d.ts` declaration, a file whose
//! export is toggled) takes a seeded random sequence of edits: change a prop's type, add or remove a prop, rename an
//! exported type, change an import path, toggle an export, delete a file and put it back, break a file and fix it. After
//! every edit the eager session's output, serialized as the CLI's canonical JSON with `stats` dropped, and the RDT
//! output, equal a fresh `extract()` of the directory with no cache; a lazy session that was asked for every component
//! before the edit gives each component exactly the cold entry. A failure names the seed, the step and every edit so far,
//! so it replays: `SEED=<n> cargo nextest run -p caliper-docgen-core session_equals_cold`.

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, WatchSession, extract, rdt};
use caliper_docgen_core::types::ExtractionOutput;
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

const SEEDS: u64 = 20;
const EDITS: usize = 12;

/// A small deterministic generator: the model is reproducible without a dependency.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        usize::try_from((self.0 >> 33) % n as u64).unwrap()
    }
}

/// The project's files. Each is rendered from the model's state, so an edit changes one field and rewrites one file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum File {
    /// The props type `Shared<n>`, with `size` and a number of extra members.
    Types,
    /// Re-exports the props type from `types.ts`.
    Index,
    /// `Button`, whose props extend the imported type.
    Button,
    /// `Card`, whose props are the local declaration's type.
    Card,
    /// `Fancy` (a `forwardRef`) and `Memoed` (a `memo`), each wrapping one of the components.
    Wrapped,
    /// A `.d.ts` that declares `Local`, the props type of `Card`.
    Local,
    /// `Toggle`, whose `export` comes and goes.
    Toggle,
}

const FILES: [File; 7] = [File::Types, File::Index, File::Button, File::Card, File::Wrapped, File::Local, File::Toggle];

impl File {
    fn name(self) -> &'static str {
        match self {
            File::Types => "types.ts",
            File::Index => "index.ts",
            File::Button => "Button.tsx",
            File::Card => "Card.tsx",
            File::Wrapped => "Wrapped.tsx",
            File::Local => "local.d.ts",
            File::Toggle => "Toggle.tsx",
        }
    }
}

#[derive(Clone)]
struct State {
    /// The name `types.ts` exports its props type as, `Shared<n>`.
    types_name: usize,
    /// The name `index.ts` re-exports, and the name `Button` imports.
    index_name: usize,
    button_name: usize,
    /// `size`'s type in `types.ts`.
    size_is_number: bool,
    /// Extra members of the shared type, and of `Button`'s own props.
    shared_extra: usize,
    own_extra: usize,
    /// Whether `Button` imports its type through `./index` or straight from `./types`.
    button_via_index: bool,
    /// Which component `Fancy` wraps, and whether `Memoed` wraps `Button`.
    fancy_wraps_card: bool,
    /// `Local`'s members, and `Card`'s own prop type.
    local_extra: usize,
    card_label_is_number: bool,
    toggle_exported: bool,
    button_exported: bool,
    exists: [bool; 7],
    broken: [bool; 7],
}

impl State {
    fn new() -> State {
        State {
            types_name: 0,
            index_name: 0,
            button_name: 0,
            size_is_number: false,
            shared_extra: 1,
            own_extra: 1,
            button_via_index: true,
            fancy_wraps_card: false,
            local_extra: 1,
            card_label_is_number: false,
            toggle_exported: true,
            button_exported: true,
            exists: [true; 7],
            broken: [false; 7],
        }
    }

    fn render(&self, file: File) -> String {
        let members =
            |prefix: &str, n: usize| -> String { (0..n).map(|i| format!("{prefix}{i}?: string; ")).collect() };
        let mut source = match file {
            File::Types => format!(
                "export interface Shared{} {{ size?: {}; {}}}\nexport type Tone = 'a' | 'b';\n",
                self.types_name,
                if self.size_is_number { "number" } else { "string" },
                members("m", self.shared_extra),
            ),
            File::Index => format!(
                "export type {{ Shared{0} }} from './types';\nexport type {{ Tone }} from './types';\n",
                self.index_name
            ),
            File::Button => format!(
                "import type {{ Shared{name} }} from '{path}';\n\
                 export interface ButtonProps extends Shared{name} {{ onClick?: () => void; {own}}}\n\
                 {export}function Button(props: ButtonProps) {{ return <button>{{props.size}}</button>; }}\n",
                name = self.button_name,
                path = if self.button_via_index { "./index" } else { "./types" },
                own = members("own", self.own_extra),
                export = if self.button_exported { "export " } else { "" },
            ),
            File::Card => format!(
                "import type {{ Local }} from './local';\n\
                 export interface CardProps extends Local {{ label?: {}; tone?: 'a' | 'b' }}\n\
                 export const Card = (props: CardProps) => <section>{{props.label}}</section>;\n",
                if self.card_label_is_number { "number" } else { "string" },
            ),
            File::Wrapped => format!(
                "import * as React from 'react';\n\
                 import {{ Button }} from './Button';\n\
                 import {{ Card }} from './Card';\n\
                 export const Fancy = React.forwardRef<HTMLElement, React.ComponentProps<typeof {}>>((props, ref) => null);\n\
                 export const Memoed = React.memo(Button);\n",
                if self.fancy_wraps_card { "Card" } else { "Button" },
            ),
            File::Local => format!("export interface Local {{ {}}}\n", members("l", self.local_extra)),
            File::Toggle => format!(
                "interface ToggleProps {{ on?: boolean }}\n{}function Toggle(props: ToggleProps) {{ return <i>{{String(props.on)}}</i>; }}\n",
                if self.toggle_exported { "export " } else { "" },
            ),
        };
        if self.broken[index(file)] {
            source.push_str("export const = {{{ ;\n");
        }
        source
    }
}

fn index(file: File) -> usize {
    FILES.iter().position(|candidate| *candidate == file).unwrap()
}

/// Applies one random edit to the state and the files, and returns the file it touched and a name for the edit.
fn edit(state: &mut State, root: &Utf8Path, rng: &mut Lcg) -> (File, String) {
    let touch = |state: &State, file: File| {
        let path = root.join(file.name());
        if state.exists[index(file)] {
            std::fs::write(&path, state.render(file)).unwrap();
        } else {
            // A file already gone (a change to a deleted file's model state) stays gone.
            drop(std::fs::remove_file(&path));
        }
    };
    let (file, what) = match rng.below(9) {
        0 => {
            // A prop's type changes: the shared type's `size`, or `Card`'s `label`.
            if rng.below(2) == 0 {
                state.size_is_number = !state.size_is_number;
                (File::Types, "change size's type")
            } else {
                state.card_label_is_number = !state.card_label_is_number;
                (File::Card, "change label's type")
            }
        }
        1 => {
            // A prop comes or goes: a member of the shared type, `Button`'s own, or the declaration's.
            match rng.below(3) {
                0 => {
                    state.shared_extra = (state.shared_extra + 1) % 3;
                    (File::Types, "add or remove a shared member")
                }
                1 => {
                    state.own_extra = (state.own_extra + 1) % 3;
                    (File::Button, "add or remove an own prop")
                }
                _ => {
                    state.local_extra = (state.local_extra + 1) % 3;
                    (File::Local, "add or remove a declared member")
                }
            }
        }
        2 => {
            // The exported type is renamed in one of the three files that name it.
            match rng.below(3) {
                0 => {
                    state.types_name = (state.types_name + 1) % 2;
                    (File::Types, "rename the type where it is declared")
                }
                1 => {
                    state.index_name = (state.index_name + 1) % 2;
                    (File::Index, "rename the type where it is re-exported")
                }
                _ => {
                    state.button_name = (state.button_name + 1) % 2;
                    (File::Button, "rename the type where it is imported")
                }
            }
        }
        3 => {
            // Delete a file that exists.
            let alive: Vec<File> = FILES.iter().copied().filter(|file| state.exists[index(*file)]).collect();
            let file = alive[rng.below(alive.len())];
            state.exists[index(file)] = false;
            (file, "delete a file")
        }
        4 => {
            // Put a deleted file back, or, with none gone, touch one unchanged.
            let gone: Vec<File> = FILES.iter().copied().filter(|file| !state.exists[index(*file)]).collect();
            if gone.is_empty() {
                (FILES[rng.below(FILES.len())], "rewrite a file unchanged")
            } else {
                let file = gone[rng.below(gone.len())];
                state.exists[index(file)] = true;
                (file, "re-add a file")
            }
        }
        5 => {
            if rng.below(2) == 0 {
                state.button_via_index = !state.button_via_index;
                (File::Button, "change an import path")
            } else {
                state.fancy_wraps_card = !state.fancy_wraps_card;
                (File::Wrapped, "change the wrapped component")
            }
        }
        6 => {
            if rng.below(2) == 0 {
                state.toggle_exported = !state.toggle_exported;
                (File::Toggle, "toggle an export")
            } else {
                state.button_exported = !state.button_exported;
                (File::Button, "toggle an export")
            }
        }
        _ => {
            // Break a file, or fix a broken one.
            let file = FILES[rng.below(FILES.len())];
            state.broken[index(file)] = !state.broken[index(file)];
            (file, if state.broken[index(file)] { "make a file unparsable" } else { "fix a file" })
        }
    };
    touch(state, file);
    (file, format!("{what} ({})", file.name()))
}

fn options(root: &Utf8Path) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![root.to_owned()],
        use_cache: false,
        html_attributes: HtmlAttributeMode::None,
        ..PipelineOptions::default()
    }
}

/// The CLI's canonical JSON (`serde_json::to_string_pretty` of the output), without the run's `stats`, parsed.
fn canonical(output: &ExtractionOutput) -> Value {
    let mut value: Value = serde_json::from_str(&serde_json::to_string_pretty(output).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("stats");
    value
}

fn rdt_json(output: &ExtractionOutput) -> Value {
    serde_json::from_str(&rdt::serialize(output, true).unwrap()).unwrap()
}

fn same(watched: &ExtractionOutput, cold: &ExtractionOutput, context: &str) {
    let (watched, cold_json) = (canonical(watched), canonical(cold));
    // Section by section, so a failure prints the component that differs rather than the whole output.
    for section in ["components", "enums", "diagnostics"] {
        let (a, b) = (&watched[section], &cold_json[section]);
        if let (Some(a), Some(b)) = (a.as_object(), b.as_object()) {
            assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>(), "{section} keys: {context}");
            for (key, expected) in b {
                assert_eq!(&a[key], expected, "{section}[{key}]: {context}");
            }
        }
        assert_eq!(a, b, "{section}: {context}");
    }
    assert_eq!(watched, cold_json, "canonical JSON: {context}");
}

fn seeds() -> Vec<u64> {
    match std::env::var("SEED").ok().and_then(|seed| seed.parse().ok()) {
        Some(seed) => vec![seed],
        None => (1..=SEEDS).collect(),
    }
}

#[test]
fn session_equals_cold_after_any_edits() {
    let (mut moved, mut steps, mut membership) = (0, 0, 0);
    for seed in seeds() {
        let dir = tempfile::Builder::new().prefix("SessionEqualsCold").tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let mut rng = Lcg(seed);
        let mut state = State::new();
        for file in FILES {
            std::fs::write(root.join(file.name()), state.render(file)).unwrap();
        }
        let options = options(&root);
        let eager = WatchSession::new(options.clone());
        let initial = eager.initialize();
        let lazy = WatchSession::lazy(options.clone());
        drop(lazy.initialize());
        let mut previous = extract(&options);
        same(&initial, &previous, &format!("seed {seed}, before any edit"));
        // The lazy session is asked for everything, so each edit has kept answers to go stale.
        for key in previous.components.keys() {
            drop(lazy.component(key));
        }
        let mut history: Vec<String> = Vec::new();
        for step in 1..=EDITS {
            let (file, label) = edit(&mut state, &root, &mut rng);
            history.push(label);
            let context = format!("seed {seed}, step {step}, edits {history:?}");
            let path = root.join(file.name());
            drop(eager.update_file(&path));
            let delta = lazy.update_file(&path);
            let cold = extract(&options);

            let watched = eager.snapshot();
            same(&watched, &cold, &context);
            assert_eq!(rdt_json(&watched), rdt_json(&cold), "rdt: {context}");

            let mut lazy_keys = lazy.component_keys();
            lazy_keys.sort();
            assert_eq!(lazy_keys, cold.components.keys().cloned().collect::<Vec<_>>(), "lazy keys: {context}");
            for (key, expected) in &cold.components {
                let answer = lazy.component(key).unwrap_or_else(|| panic!("lazy: no {key}: {context}"));
                assert_eq!(&answer.entry, expected, "lazy {key}: {context}");
            }
            // A lazy delta names every component whose entry changed.
            for (key, entry) in &cold.components {
                if previous.components.get(key) != Some(entry) {
                    assert!(
                        delta.updated_keys.contains(key),
                        "lazy delta misses {key}: {:?}, {context}",
                        delta.updated_keys
                    );
                }
            }
            // The model is only as strong as the evidence it moves.
            moved += usize::from(canonical(&previous) != canonical(&cold));
            membership += usize::from(!previous.components.keys().eq(cold.components.keys()));
            steps += 1;
            previous = cold;
        }
    }
    let total = seeds().len() * EDITS;
    assert_eq!(steps, total);
    // Many edits change the output, and some add or remove a component (a deleted or broken file, a toggled export);
    // without a seed pinned, both must be common or the model is not exercising what it claims.
    if total == usize::try_from(SEEDS).unwrap() * EDITS {
        assert!(moved * 2 >= total, "only {moved} of {total} edits changed the output");
        assert!(membership * 8 >= total, "only {membership} of {total} edits changed the set of components");
    }
}
