//! Rust text for JavaScript strings, which may hold a lone surrogate that UTF-8 cannot.

use std::borrow::Cow;

use oxc_ast::StaticPropertyName;
use oxc_str::JSStr;

/// The text of a string literal. A lone surrogate becomes U+FFFD, which is what OXC stored for it before it kept such
/// strings losslessly, so output for the same source does not change.
pub(super) fn lossy(value: JSStr<'_>) -> Cow<'_, str> {
    match value.as_str() {
        Some(text) => Cow::Borrowed(text),
        None => Cow::Owned(String::from_utf16_lossy(&value.encode_utf16().collect::<Vec<_>>())),
    }
}

/// The name a property key spells, in the same lossy text as [`lossy`].
pub(super) fn property_name(name: &StaticPropertyName<'_>) -> String {
    lossy(name.as_js_str()).into_owned()
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_ast::StaticPropertyName;
    use oxc_str::JSStrBuilder;

    use super::*;

    fn with_units<'a>(allocator: &'a Allocator, units: &[u16]) -> JSStr<'a> {
        let mut builder = JSStrBuilder::new_in(&allocator);
        builder.push_utf16(units);
        builder.into_js_str()
    }

    #[test]
    fn well_formed_text_is_borrowed_unchanged() {
        assert_eq!(lossy(JSStr::from("size \u{e9}\u{1f600}")), Cow::Borrowed("size \u{e9}\u{1f600}"));
    }

    #[test]
    fn a_lone_surrogate_becomes_the_replacement_character() {
        let allocator = Allocator::new();
        assert_eq!(lossy(with_units(&allocator, &[0x61, 0xD800, 0x62])), "a\u{fffd}b");
        assert_eq!(lossy(with_units(&allocator, &[0xDC00])), "\u{fffd}");
    }

    #[test]
    fn a_surrogate_pair_is_one_character() {
        let allocator = Allocator::new();
        assert_eq!(lossy(with_units(&allocator, &[0xD83D, 0xDE00])), "\u{1f600}");
    }

    #[test]
    fn property_names_use_the_same_text() {
        let allocator = Allocator::new();
        assert_eq!(property_name(&StaticPropertyName::from("size")), "size");
        assert_eq!(property_name(&StaticPropertyName::Owned("1.5".into())), "1.5");
        assert_eq!(property_name(&StaticPropertyName::Borrowed(with_units(&allocator, &[0xD800]))), "\u{fffd}");
    }
}
