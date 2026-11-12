//! The non-fatal issues extraction reports instead of failing: what a [`Diagnostic`] carries, how serious it is, and
//! the codes a program can match on.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A non-fatal issue discovered during extraction.
///
/// Extraction never fails on a bad file or type: it reports what it could not do as a diagnostic and carries on with
/// the rest, so a consumer reads the `diagnostics` of the [`ExtractionOutput`](crate::types::output::ExtractionOutput)
/// next to its components. The worst [`severity`](Self::severity) decides the CLI's exit code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// How serious the issue is; see [`DiagnosticSeverity`].
    pub severity: DiagnosticSeverity,
    /// What happened, in a sentence a person can read without the code: it names the type, file or component involved.
    pub message: String,
    /// The file the issue is about, as the path the pipeline discovered it under a source directory. `None` for an
    /// issue about the run as a whole.
    pub file: Option<String>,
    /// One-based line in [`file`](Self::file), when the issue has a position. Few diagnostics do: most are about a
    /// type or component, not a span.
    pub line: Option<u32>,
    /// One-based column on [`line`](Self::line); set together with it by [`Diagnostic::with_position`].
    pub column: Option<u32>,
    /// A suggested next step for the person reading the diagnostic, when one exists.
    pub help: Option<String>,
    /// A stable code a program can match on, where [`message`](Self::message) is for people and may be reworded.
    pub code: DiagnosticCode,
    /// Output keys of the components whose resolution reported this diagnostic, sorted. Empty for diagnostics about a
    /// file or the run as a whole. Identical diagnostics from many components are reported once, listing them all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects: Vec<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{file}: ")?;
        }
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Diagnostic {}

impl Diagnostic {
    /// A diagnostic with no file, position or help; add them with the `with_*` methods.
    pub fn new(severity: DiagnosticSeverity, code: DiagnosticCode, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity,
            message: message.into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code,
            affects: Vec::new(),
        }
    }

    /// Names the file the diagnostic is about.
    #[must_use]
    pub fn with_file(mut self, file: impl Into<String>) -> Diagnostic {
        self.file = Some(file.into());
        self
    }

    /// One-based line and column.
    #[must_use]
    pub fn with_position(mut self, line: u32, column: u32) -> Diagnostic {
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    /// Adds a suggested next step for the reader.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Diagnostic {
        self.help = Some(help.into());
        self
    }

    /// A file could not be read from disk (permission error, race with deletion, etc).
    pub fn io_read_error(path: &camino::Utf8Path, error: &std::io::Error) -> Diagnostic {
        Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("Failed to read '{path}': {error}"),
            file: Some(path.to_string()),
            line: None,
            column: None,
            help: Some("Check file permissions and that the file exists.".into()),
            code: DiagnosticCode::IoError,
            affects: Vec::new(),
        }
    }
}

/// How serious a [`Diagnostic`] is. It sets the exit code of the CLI commands (see
/// [`ExtractionOutput::exit_code`](crate::types::output::ExtractionOutput::exit_code)); the declaration order is not a
/// ranking.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSeverity {
    /// Something could not be read or extracted, so the output is missing or wrong for it. Exit code `2`.
    Error,
    /// The output is complete but part of it is a guess, or an input looked wrong. Exit code `1` under `--strict`,
    /// otherwise `0`.
    Warning,
    /// Worth knowing, and no action is needed: a type that was left opaque by design, a cache that was discarded. Exit
    /// code `0`.
    Info,
}

/// Machine-readable diagnostic codes for programmatic consumers.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticCode {
    /// A type named in a component's props could not be found: its import does not resolve to a file under the source
    /// directories, or its package declaration was not loaded. The prop keeps the name as a reference.
    UnresolvableImport,
    /// A generic type alias/interface was referenced with fewer type arguments
    /// than it declares parameters — the trailing, unfilled parameters were
    /// left as bare unresolved names in the substituted body.
    GenericArgumentMismatch,
    /// A type that cannot be expanded statically (an unbound generic, a type from a library the tool does not model)
    /// appears in the output as opaque, with the reason on the type itself.
    OpaqueType,
    /// A type nests deeper than the extractor or resolver follows, so the rest of it is left as written (extractor)
    /// or opaque (resolver). Distinct from [`CircularReference`](Self::CircularReference), where the type loops.
    MaxDepthExceeded,
    /// Deliberately unconstructed within this crate — reserved headroom for
    /// external consumers synthesizing their own [`Diagnostic`] outside the
    /// extraction pipeline (e.g. a wrapping tool reporting its own issue
    /// through this crate's diagnostic shape). Do not remove for being
    /// "unused"; nothing in `crates/core` or `crates/cli` is expected to
    /// construct it.
    Unknown,
    /// JSDoc @default conflicts with code default value — code value was used.
    JsDocDefaultMismatch,
    /// Indexed access type (Type["key"]) that could not be resolved from known tables.
    IndexedAccessOpaque,
    /// Template literal type that could not be statically expanded to its strings: it is kept as a pattern
    /// (`PropType::TemplateLiteral`), which lists no values. The name predates the pattern, which is not opaque.
    TemplateLiteralOpaque,
    /// Discriminated union detected — props merged with discriminant surfaced.
    DiscriminatedUnion,
    /// File could not be read — permission error or file missing.
    IoError,
    /// Source file exceeds the maximum type-nesting depth; skipped to avoid parser stack overflow.
    ExcessiveNesting,
    /// TypeScript syntax error reported by the parser.
    ParseError,
    /// An internal panic was caught and converted into a diagnostic instead
    /// of crashing the process (see ADR 0005). Never expected in normal
    /// operation — always a bug, filed with the panic's own message.
    InternalPanic,
    /// The extractor recognized an AST shape as a candidate (a type-alias utility
    /// invocation, a component-detector pattern) but it was malformed or
    /// incomplete in a way that made it unsupported — distinct from "wrong shape,
    /// not a candidate at all," which emits no diagnostic.
    SkippedCandidate,
    /// Two resolved components produced the identical output key (same display
    /// name, same disambiguated path suffix) — the later one silently overwrote
    /// the earlier in `ExtractionOutput::components`. Seen with 3+ same-named
    /// declarations in one file, or with overlapping `src_dirs` configs.
    ComponentKeyCollision,
    /// Two source files export an enum with the same name. The public `enums` map is keyed by bare name, so only the
    /// one from the lexicographically first file path is kept.
    DuplicateEnumName,
    /// A type references itself, directly or through other types; resolution stopped at the cycle. Distinct from
    /// `MaxDepthExceeded`, which means the nesting was merely too deep.
    CircularReference,
    /// `@types/react` or a TypeScript `lib.*.d.ts` file the run needs could not be found, as opposed to a type or
    /// import in the user's own code that failed to resolve.
    MissingTypeLibrary,
    /// The experimental TypeScript checker backend (`--experimental-semantic-types`) could not answer for the run or
    /// for a component, so the structural result stands. The message says why.
    SemanticBackend,
}

impl DiagnosticCode {
    /// What becomes of a structural diagnostic with this code about a component once the TypeScript checker has
    /// answered for that component. Each code is decided here, so a new code cannot be added without a decision; the
    /// reasons are in `docs/type-checker-integration.md` ("Diagnostics a checker answer supersedes").
    pub fn superseded_by_checker(&self) -> Superseded {
        match self {
            DiagnosticCode::UnresolvableImport => Superseded::UnlessUnresolvedByChecker,
            DiagnosticCode::GenericArgumentMismatch
            | DiagnosticCode::OpaqueType
            | DiagnosticCode::MaxDepthExceeded
            | DiagnosticCode::IndexedAccessOpaque
            | DiagnosticCode::TemplateLiteralOpaque
            | DiagnosticCode::CircularReference => Superseded::Yes,
            DiagnosticCode::Unknown
            | DiagnosticCode::JsDocDefaultMismatch
            | DiagnosticCode::DiscriminatedUnion
            | DiagnosticCode::IoError
            | DiagnosticCode::ExcessiveNesting
            | DiagnosticCode::ParseError
            | DiagnosticCode::InternalPanic
            | DiagnosticCode::SkippedCandidate
            | DiagnosticCode::ComponentKeyCollision
            | DiagnosticCode::DuplicateEnumName
            | DiagnosticCode::MissingTypeLibrary
            | DiagnosticCode::SemanticBackend => Superseded::No,
        }
    }
}

/// What becomes of a structural diagnostic about a component once the TypeScript checker has answered for that
/// component ([`DiagnosticCode::superseded_by_checker`]). The diagnostic names the component, not the prop, so a
/// superseded one goes only while nothing it could be about is left: it stays while a structural prop the checker did
/// not return still holds a part the structural pass could not read.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Superseded {
    /// About something the checker's answer does not replace (a file, the run, a default value, a discriminant, an
    /// inheritance layer, the checker itself): it stays.
    No,
    /// About how the structural pass read the props' types, which the checker's answer replaces: it goes.
    Yes,
    /// About a name the structural pass could not resolve: it goes, unless a prop of the merged answer still holds a
    /// name the checker could not resolve either (`degradedBy` lists `unresolvableImport`), the same fault of the project
    /// seen by both passes.
    UnlessUnresolvedByChecker,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every code with the decision `docs/type-checker-integration.md` gives it. `listed` has no wildcard arm, so a new
    /// code fails to compile here until it is listed with its decision.
    #[test]
    fn every_code_has_an_explicit_decision_about_a_checker_answer() {
        fn listed(code: &DiagnosticCode) -> usize {
            match code {
                DiagnosticCode::UnresolvableImport => 0,
                DiagnosticCode::GenericArgumentMismatch => 1,
                DiagnosticCode::OpaqueType => 2,
                DiagnosticCode::MaxDepthExceeded => 3,
                DiagnosticCode::Unknown => 4,
                DiagnosticCode::JsDocDefaultMismatch => 5,
                DiagnosticCode::IndexedAccessOpaque => 6,
                DiagnosticCode::TemplateLiteralOpaque => 7,
                DiagnosticCode::DiscriminatedUnion => 8,
                DiagnosticCode::IoError => 9,
                DiagnosticCode::ExcessiveNesting => 10,
                DiagnosticCode::ParseError => 11,
                DiagnosticCode::InternalPanic => 12,
                DiagnosticCode::SkippedCandidate => 13,
                DiagnosticCode::ComponentKeyCollision => 14,
                DiagnosticCode::DuplicateEnumName => 15,
                DiagnosticCode::CircularReference => 16,
                DiagnosticCode::MissingTypeLibrary => 17,
                DiagnosticCode::SemanticBackend => 18,
            }
        }
        let decisions = [
            (DiagnosticCode::UnresolvableImport, Superseded::UnlessUnresolvedByChecker),
            (DiagnosticCode::GenericArgumentMismatch, Superseded::Yes),
            (DiagnosticCode::OpaqueType, Superseded::Yes),
            (DiagnosticCode::MaxDepthExceeded, Superseded::Yes),
            (DiagnosticCode::Unknown, Superseded::No),
            (DiagnosticCode::JsDocDefaultMismatch, Superseded::No),
            (DiagnosticCode::IndexedAccessOpaque, Superseded::Yes),
            (DiagnosticCode::TemplateLiteralOpaque, Superseded::Yes),
            (DiagnosticCode::DiscriminatedUnion, Superseded::No),
            (DiagnosticCode::IoError, Superseded::No),
            (DiagnosticCode::ExcessiveNesting, Superseded::No),
            (DiagnosticCode::ParseError, Superseded::No),
            (DiagnosticCode::InternalPanic, Superseded::No),
            (DiagnosticCode::SkippedCandidate, Superseded::No),
            (DiagnosticCode::ComponentKeyCollision, Superseded::No),
            (DiagnosticCode::DuplicateEnumName, Superseded::No),
            (DiagnosticCode::CircularReference, Superseded::Yes),
            (DiagnosticCode::MissingTypeLibrary, Superseded::No),
            (DiagnosticCode::SemanticBackend, Superseded::No),
        ];

        let positions: Vec<usize> = decisions.iter().map(|(code, _)| listed(code)).collect();
        assert_eq!(positions, (0..decisions.len()).collect::<Vec<_>>(), "each code once, in declaration order");
        for (code, decision) in decisions {
            assert_eq!(code.superseded_by_checker(), decision, "{code:?}");
        }
    }

    #[test]
    fn io_read_error_reports_the_path_and_underlying_error() {
        let path = camino::Utf8Path::new("src/Button.tsx");
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");

        let diagnostic = Diagnostic::io_read_error(path, &error);

        assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
        assert_eq!(diagnostic.code, DiagnosticCode::IoError);
        assert_eq!(diagnostic.file.as_deref(), Some("src/Button.tsx"));
        assert!(diagnostic.message.contains("src/Button.tsx"));
        assert!(diagnostic.message.contains("permission denied"));
    }

    #[test]
    fn internal_panic_code_serializes_as_screaming_snake_case() {
        let diagnostic = Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "panic in rayon worker".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: DiagnosticCode::InternalPanic,
            affects: Vec::new(),
        };
        let json = serde_json::to_string(&diagnostic).unwrap();
        assert!(json.contains("\"INTERNAL_PANIC\""), "expected INTERNAL_PANIC in {json}");
    }

    #[test]
    fn skipped_candidate_code_serializes_screaming_snake_case() {
        let json = serde_json::to_string(&DiagnosticCode::SkippedCandidate).unwrap();
        assert_eq!(json, "\"SKIPPED_CANDIDATE\"");
    }
}
