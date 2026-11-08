//! What the checker prints is where two facts live that its structured answers do not state: whether a parameter is
//! optional (`name?: T`), and what a tuple element is labeled (`head: string`). Both are node-level facts that the API
//! reaches only through a declaration handle, which names a node by its index in the source file and so cannot be
//! read without decoding that file. The readers here take them out of the printed type, and refuse any text that does
//! not read to its end (a long type is cut short with `...`), so a fact is missing rather than invented. A caller that
//! gets no fact back must not stand in for it: a parameter is not required because its marker could not be read.

/// How the checker prints one parameter of a function type: `name?: T` is optional, `...name: T[]` is the rest
/// parameter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Marker {
    pub optional: bool,
    pub rest: bool,
}

/// The markers of the parameters of a function type as the checker prints it, `(percent?: number, ...rest: string[])
/// => string`, one per parameter in order. `None` when the text is not a parameter list that can be read to its end
/// (a long type is cut short with `...`).
pub(crate) fn parameter_markers(text: &str) -> Option<Vec<Marker>> {
    let chars: Vec<char> = text.chars().collect();
    // Type parameters come first: `<T>(value: T) => T`.
    let open = if chars.first() == Some(&'<') { matching_close(&chars, 0)? + 1 } else { 0 };
    if chars.get(open) != Some(&'(') {
        return None;
    }
    let close = matching_close(&chars, open)?;
    let list = chars.get(open + 1..close)?;
    if list.iter().all(|c| c.is_whitespace()) {
        return Some(Vec::new());
    }
    let mut segments = top_level_segments(list, ',')?;
    // A `this` parameter is printed but is not one of the signature's parameters.
    if segments.first().is_some_and(|first| is_this_parameter(first)) {
        segments.remove(0);
    }
    segments.into_iter().map(parameter_marker).collect()
}

fn is_this_parameter(parameter: &[char]) -> bool {
    let text: String = parameter.iter().collect();
    text.trim_start().strip_prefix("this").is_some_and(|after| after.trim_start().starts_with(':'))
}

/// The markers of every signature in a printed function, constructor or object type, calls and constructs apart, each in
/// the order the checker lists its signatures: `(a?: 1) => 2`, `new (a?: 1) => X`, `{ (a?: 1): 2; (b: 3): 4; x: 5 }`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SignatureMarkers {
    pub calls: Vec<Vec<Marker>>,
    pub constructs: Vec<Vec<Marker>>,
}

/// `None` when the text is not a type that states its signatures to the end (a signature it cannot read, or text cut
/// short with `...`).
pub(crate) fn signature_markers(text: &str) -> Option<SignatureMarkers> {
    let text = text.trim();
    let mut found = SignatureMarkers::default();
    if !text.starts_with('{') {
        let (construct, markers) = signature_of(text)??;
        found.push(construct, markers);
        return Some(found);
    }
    let chars: Vec<char> = text.chars().collect();
    if matching_close(&chars, 0)? + 1 != chars.len() {
        return None;
    }
    let body = chars.get(1..chars.len() - 1)?;
    if body.iter().all(|c| c.is_whitespace()) {
        return Some(found);
    }
    for member in top_level_segments(body, ';')? {
        let member: String = member.iter().collect();
        // A property, method or index signature is not a signature of the type.
        let Some(signature) = signature_of(member.trim()) else { continue };
        let (construct, markers) = signature?;
        found.push(construct, markers);
    }
    Some(found)
}

impl SignatureMarkers {
    fn push(&mut self, construct: bool, markers: Vec<Marker>) {
        if construct { self.constructs.push(markers) } else { self.calls.push(markers) }
    }
}

/// `None` for text that does not start a signature, `Some(None)` for one whose parameters cannot be read; otherwise
/// whether it is a construct signature, and its markers.
fn signature_of(text: &str) -> Option<Option<(bool, Vec<Marker>)>> {
    let (construct, rest) = match text.strip_prefix("new ") {
        Some(rest) => (true, rest.trim_start()),
        None => (false, text),
    };
    if !rest.starts_with(['(', '<']) {
        return None;
    }
    Some(parameter_markers(rest).map(|markers| (construct, markers)))
}

fn parameter_marker(parameter: &[char]) -> Option<Marker> {
    let text: String = parameter.iter().collect();
    let text = text.trim_start();
    let (rest, text) = match text.strip_prefix("...") {
        Some(after) => (true, after),
        None => (false, text),
    };
    // The name: an identifier, or `__0` for a destructured parameter. Whatever follows it, `?` marks it optional.
    let after_name = text.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '$');
    if after_name.len() == text.len() {
        return None;
    }
    Some(Marker { optional: after_name.starts_with('?'), rest })
}

/// The labels of a tuple type as the checker prints it, `[x: number, y?: string]`, one per element in order. `None`
/// when the text is not a tuple that can be read to its end, or does not have `count` elements.
pub(crate) fn tuple_labels(text: &str, count: usize) -> Option<Vec<String>> {
    let text = text.trim();
    let text = text.strip_prefix("readonly ").unwrap_or(text);
    let chars: Vec<char> = text.chars().collect();
    if chars.first() != Some(&'[') || matching_close(&chars, 0)? + 1 != chars.len() {
        return None;
    }
    let list = chars.get(1..chars.len() - 1)?;
    let labels: Vec<String> = top_level_segments(list, ',')?.into_iter().map(tuple_label).collect::<Option<_>>()?;
    (labels.len() == count).then_some(labels)
}

fn tuple_label(element: &[char]) -> Option<String> {
    let text: String = element.iter().collect();
    let text = text.trim_start();
    let text = text.strip_prefix("...").unwrap_or(text);
    let name_len = text.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))?;
    let (name, after) = text.split_at(name_len);
    // `name: T` and `name?: T`; a bare `T?` is an unlabeled optional element.
    let after = after.strip_prefix('?').unwrap_or(after);
    (!name.is_empty() && after.starts_with(':')).then(|| name.to_string())
}

/// The parts of a list between its top-level `separator`s.
fn top_level_segments(list: &[char], separator: char) -> Option<Vec<&[char]>> {
    let mut segments = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < list.len() {
        match list.get(i)? {
            '"' | '\'' | '`' => i = skip_string(list, i)?,
            '(' | '[' | '{' | '<' => i = matching_close(list, i)?,
            '=' if list.get(i + 1) == Some(&'>') => i += 1,
            c if *c == separator => {
                segments.push(list.get(start..i)?);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    segments.push(list.get(start..)?);
    Some(segments)
}

/// The index of the bracket that closes the one at `open`, skipping strings and the `>` of `=>`.
fn matching_close(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < chars.len() {
        match chars.get(i)? {
            '"' | '\'' | '`' => i = skip_string(chars, i)?,
            '=' if chars.get(i + 1) == Some(&'>') => i += 1,
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The index of the quote that closes the string, or template literal, that starts at `open`.
fn skip_string(chars: &[char], open: usize) -> Option<usize> {
    let quote = *chars.get(open)?;
    let mut i = open + 1;
    while i < chars.len() {
        match chars.get(i)? {
            '\\' => i += 1,
            c if *c == quote => return Some(i),
            '$' if quote == '`' && chars.get(i + 1) == Some(&'{') => i = matching_close(chars, i + 1)?,
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: Marker = Marker { optional: false, rest: false };
    const OPTIONAL: Marker = Marker { optional: true, rest: false };
    const REST: Marker = Marker { optional: false, rest: true };

    #[test]
    fn a_printed_signature_gives_one_marker_per_parameter() {
        let cases: [(&str, Vec<Marker>); 6] = [
            ("() => void", vec![]),
            ("(a: number) => void", vec![REQUIRED]),
            ("(percent?: number, ...rest: string[]) => string", vec![OPTIONAL, REST]),
            ("(...args: any[]) => void", vec![REST]),
            ("(a: number, b?: string) => void", vec![REQUIRED, OPTIONAL]),
            // `undefined` in the type is not the `?` marker.
            ("(a: number, b: string | undefined) => void", vec![REQUIRED, REQUIRED]),
        ];
        for (text, expected) in cases {
            assert_eq!(parameter_markers(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn markers_inside_a_parameter_type_are_not_the_parameters() {
        let cases: [(&str, Vec<Marker>); 5] = [
            ("(handler: (a?: number, ...b: string[]) => void, c?: string) => void", vec![REQUIRED, OPTIONAL]),
            ("(options: { a?: number; b: string }, c?: boolean) => void", vec![REQUIRED, OPTIONAL]),
            ("(pair: [first?: string, second?: number], ...more: Array<string>) => void", vec![REQUIRED, REST]),
            // Commas and brackets inside a string literal or a template literal type.
            ("(key: \"a,b\" | 'c)', template?: `--${string}`) => void", vec![REQUIRED, OPTIONAL]),
            ("(map: Map<string, Array<number>>, last?: number) => void", vec![REQUIRED, OPTIONAL]),
        ];
        for (text, expected) in cases {
            assert_eq!(parameter_markers(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn type_parameters_and_destructured_parameters_are_skipped_over() {
        assert_eq!(parameter_markers("<T>(value: T, key?: keyof T) => T"), Some(vec![REQUIRED, OPTIONAL]));
        assert_eq!(parameter_markers("<T extends { a?: 1 }, U = (x?: 1) => 2>(v: T) => U"), Some(vec![REQUIRED]));
        assert_eq!(parameter_markers("(__0: { a: string }, __1?: number) => void"), Some(vec![REQUIRED, OPTIONAL]));
    }

    #[test]
    fn text_that_is_not_a_complete_parameter_list_has_no_markers() {
        for text in
            ["(a: number, b?: Very.Long.Type<With, Many...", "{ a: number }", "", "(a: number", "new (a?: 1) => X"]
        {
            assert_eq!(parameter_markers(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_this_parameter_is_printed_but_is_not_a_parameter() {
        assert_eq!(parameter_markers("(this: Window, a?: number) => void"), Some(vec![OPTIONAL]));
        assert_eq!(parameter_markers("(this: Window) => void"), Some(vec![]));
        // Only a first parameter named `this` is the `this` parameter.
        assert_eq!(parameter_markers("(thisArg?: number) => void"), Some(vec![OPTIONAL]));
    }

    #[test]
    fn every_signature_of_a_printed_type_has_its_own_markers() {
        let one = |construct: bool, markers: Vec<Marker>| {
            if construct {
                SignatureMarkers { calls: vec![], constructs: vec![markers] }
            } else {
                SignatureMarkers { calls: vec![markers], constructs: vec![] }
            }
        };
        assert_eq!(signature_markers("(a?: number) => void"), Some(one(false, vec![OPTIONAL])));
        assert_eq!(signature_markers("new (a: number, b?: string) => Date"), Some(one(true, vec![REQUIRED, OPTIONAL])));
        assert_eq!(
            signature_markers("{ (a?: string): string; (a: number, b?: number): number; }"),
            Some(SignatureMarkers { calls: vec![vec![OPTIONAL], vec![REQUIRED, OPTIONAL]], constructs: vec![] })
        );
        // Properties, methods and index signatures are not signatures of the type.
        assert_eq!(
            signature_markers(
                "{ <T>(v: T, w?: T): T; new (x?: 1): Date; displayName: string; m(y?: 1): void; [k: string]: (z?: 1) => void; }"
            ),
            Some(SignatureMarkers { calls: vec![vec![REQUIRED, OPTIONAL]], constructs: vec![vec![OPTIONAL]] })
        );
        assert_eq!(signature_markers("{ a: string; }"), Some(SignatureMarkers::default()));
        assert_eq!(signature_markers("{ }"), Some(SignatureMarkers::default()));
    }

    #[test]
    fn text_that_does_not_state_its_signatures_to_the_end_has_no_markers() {
        // A name, an alias that was not expanded, text cut short, a signature that cannot be read.
        for text in
            ["Handler", "Overloaded", "{ (a?: string): void; ...", "{ (a?: string", "abstract new (a?: 1) => X", ""]
        {
            assert_eq!(signature_markers(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_labeled_tuple_gives_one_label_per_element() {
        assert_eq!(tuple_labels("[x: number, y: string]", 2), Some(vec!["x".to_string(), "y".to_string()]));
        assert_eq!(
            tuple_labels("readonly [head: string, tail?: number, ...rest: boolean[]]", 3),
            Some(vec!["head".to_string(), "tail".to_string(), "rest".to_string()])
        );
        // A comma inside a label's type is not a separator.
        assert_eq!(
            tuple_labels("[pair: [a: 1, b: 2], map: Map<string, number>]", 2),
            Some(vec!["pair".to_string(), "map".to_string()])
        );
    }

    #[test]
    fn text_that_is_not_a_labeled_tuple_has_no_labels() {
        for (text, count) in [
            ("[string, number]", 2),
            ("[x: number, string]", 2),
            ("[string, number?]", 2),
            ("[x: number, y: string]", 3),
            ("[x: number, y: Very.Long...", 2),
            ("number[]", 1),
        ] {
            assert_eq!(tuple_labels(text, count), None, "{text}");
        }
    }
}
