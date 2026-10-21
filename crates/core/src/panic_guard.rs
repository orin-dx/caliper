//! Single sanctioned panic-containment boundary (see ADR 0005).
//!
//! Every panic reachable from a rayon `.map()`, a `DocgenPlugin` hook, or a
//! NAPI entry point crosses this function on its way to becoming a
//! `Diagnostic` instead of aborting a batch, killing the whole pipeline, or
//! poisoning a session lock.

use crate::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

/// Run `f`, converting a panic into `Err(Diagnostic)` tagged with `label`
/// instead of letting it unwind past this call site.
///
/// Wraps `f` in `AssertUnwindSafe` internally rather than requiring callers
/// to prove unwind-safety themselves. This codebase's data (`SourceData`,
/// `ComponentEntry`, plugin state) has no interior mutability (`Cell`,
/// `RefCell`) that could leave a torn, observable half-write behind after a
/// caught panic — the whole operation is abandoned and its output discarded
/// (or its `&mut` target left exactly as it was before the panicking call),
/// so the invariant `AssertUnwindSafe` exists to protect doesn't apply here.
pub fn contain_panic<T>(label: &str, f: impl FnOnce() -> T) -> Result<T, Diagnostic> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => Ok(value),
        Err(payload) => Err(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("{label} panicked: {}", panic_message(payload.as_ref())),
            file: None,
            line: None,
            column: None,
            help: Some("This is an internal bug — please file a report with the input that triggered it.".into()),
            code: DiagnosticCode::InternalPanic,
            affects: Vec::new(),
        }),
    }
}

/// Extract a human-readable message from a `catch_unwind` payload — panics
/// carry either a `&str` (`panic!("literal")`) or a `String`
/// (`panic!("{}", x)`); anything else has no stable text representation.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DiagnosticCode;

    #[test]
    fn contain_panic_returns_ok_when_f_does_not_panic() {
        let result = contain_panic("test", || 42);
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn contain_panic_converts_str_panic_to_diagnostic() {
        let result: Result<(), Diagnostic> = contain_panic("resolve:Button", || panic!("boom"));
        let diagnostic = result.expect_err("panic should be caught, not propagated");
        assert_eq!(diagnostic.code, DiagnosticCode::InternalPanic);
        assert!(
            diagnostic.message.contains("resolve:Button"),
            "message should carry the label, got {}",
            diagnostic.message
        );
        assert!(diagnostic.message.contains("boom"), "message should carry the panic text, got {}", diagnostic.message);
    }

    #[test]
    fn contain_panic_converts_string_panic_to_diagnostic() {
        let result: Result<(), Diagnostic> = contain_panic("parse:foo.tsx", || panic!("bad input: {}", 42));
        let diagnostic = result.expect_err("panic should be caught, not propagated");
        assert!(diagnostic.message.contains("bad input: 42"), "got {}", diagnostic.message);
    }
}
