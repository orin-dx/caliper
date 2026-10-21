use std::fmt;

use serde::{Deserialize, Serialize};

/// A non-fatal issue discovered during extraction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub help: Option<String>,
    pub code: DiagnosticCode,
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
        Diagnostic { severity, message: message.into(), file: None, line: None, column: None, help: None, code }
    }

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
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

/// Machine-readable diagnostic codes for programmatic consumers.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticCode {
    UnresolvableImport,
    /// A generic type alias/interface was referenced with fewer type arguments
    /// than it declares parameters — the trailing, unfilled parameters were
    /// left as bare unresolved names in the substituted body.
    GenericArgumentMismatch,
    OpaqueType,
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
    /// Default value is a runtime expression that could not be statically evaluated.
    ComputedDefault,
    /// Indexed access type (Type["key"]) that could not be resolved from known tables.
    IndexedAccessOpaque,
    /// Template literal type that could not be statically expanded.
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
