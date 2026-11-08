//! A checker type reduced to the existing [`PropType`], where that is lossless; otherwise the checker's own text as a
//! `Named` type. The output schema is unchanged: nothing here adds a variant.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use caliper_docgen_core::types::{FunctionParam, ObjectField, PropType};
use rustc_hash::FxHashMap;

use crate::rpc::{CallError, Rpc, SymbolInfo, TypeInfo, flag};

/// Nesting past which a type is written as the checker prints it.
const MAX_DEPTH: u8 = 4;
/// An inline object type with more fields than this is written as the checker prints it.
const MAX_INLINE_FIELDS: usize = 16;

type Cache = Mutex<FxHashMap<(u64, u8), PropType>>;
type Converted<'a> = Pin<Box<dyn Future<Output = Result<PropType, CallError>> + Send + 'a>>;

#[derive(Clone)]
pub(crate) struct Converter {
    rpc: Arc<Rpc>,
    cache: Arc<Cache>,
}

impl Converter {
    pub(crate) fn new(rpc: Arc<Rpc>) -> Self {
        Self { rpc, cache: Arc::new(Mutex::new(FxHashMap::default())) }
    }

    pub(crate) fn convert<'a>(&'a self, info: &'a TypeInfo, depth: u8) -> Converted<'a> {
        Box::pin(async move {
            let key = (info.id, depth);
            if let Some(hit) = self.cache.lock().ok().and_then(|cache| cache.get(&key).cloned()) {
                return Ok(hit);
            }
            let converted = self.uncached(info, depth).await?;
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert(key, converted.clone());
            }
            Ok(converted)
        })
    }

    async fn uncached(&self, info: &TypeInfo, depth: u8) -> Result<PropType, CallError> {
        let flags = info.flags;
        let simple = [
            (flag::ANY, PropType::Any),
            (flag::UNKNOWN, PropType::Unknown),
            (flag::UNDEFINED, PropType::Undefined),
            (flag::NULL, PropType::Null),
            (flag::VOID, PropType::Void),
            (flag::NEVER, PropType::Never),
            (flag::STRING, PropType::String),
            (flag::NUMBER, PropType::Number),
            (flag::BOOLEAN, PropType::Boolean),
            // A pattern such as `var(--${string})` or `Uppercase<string>` stands for strings.
            (flag::TEMPLATE_LITERAL | flag::STRING_MAPPING, PropType::String),
        ];
        if let Some((_, known)) = simple.into_iter().find(|(bit, _)| flags & bit != 0) {
            return Ok(known);
        }
        if flags & (flag::ENUM_LITERAL | flag::ENUM | flag::TYPE_PARAMETER) != 0 {
            return self.named(info).await;
        }
        if flags & flag::STRING_LITERAL != 0 {
            return match info.value.as_ref().and_then(|value| value.as_str()) {
                Some(text) => Ok(PropType::StringLiteral(text.to_string())),
                None => self.named(info).await,
            };
        }
        if flags & flag::NUMBER_LITERAL != 0 {
            return match info.value.as_ref().and_then(|value| value.as_f64()) {
                Some(number) => Ok(PropType::NumberLiteral(number)),
                None => self.named(info).await,
            };
        }
        // The beta's responses carry neither `value` nor `intrinsicName` for `true` and `false`.
        if flags & flag::BOOLEAN_LITERAL != 0 {
            return Ok(PropType::BoolLiteral(self.rpc.type_to_string(info.id).await? == "true"));
        }
        if depth >= MAX_DEPTH {
            return self.named(info).await;
        }
        if flags & flag::UNION != 0 {
            return self.union(info, depth).await;
        }
        if flags & flag::INTERSECTION != 0 {
            return self.intersection(info, depth).await;
        }
        if flags & flag::OBJECT != 0 {
            return self.object(info, depth).await;
        }
        self.named(info).await
    }

    async fn named(&self, info: &TypeInfo) -> Result<PropType, CallError> {
        Ok(PropType::Named { name: self.rpc.type_to_string(info.id).await?.into(), args: Vec::new() })
    }

    /// `ReactNode` is a union alias of a dozen members; the alias is what the structural result reports too.
    async fn react_alias(&self, info: &TypeInfo) -> Result<Option<PropType>, CallError> {
        let Some(SymbolInfo { name, .. }) = self.rpc.alias_symbol(info.id).await? else { return Ok(None) };
        Ok(match name.as_str() {
            "ReactNode" => Some(PropType::ReactNode),
            "ElementType" => Some(PropType::ElementType),
            _ => None,
        })
    }

    async fn members(&self, info: &TypeInfo, depth: u8) -> Result<Vec<PropType>, CallError> {
        let mut converted = Vec::new();
        for member in self.rpc.members(info.id).await? {
            converted.push(self.convert(&member, depth + 1).await?);
        }
        Ok(converted)
    }

    async fn union(&self, info: &TypeInfo, depth: u8) -> Result<PropType, CallError> {
        if let Some(known) = self.react_alias(info).await? {
            return Ok(known);
        }
        let mut flat = Vec::new();
        for member in self.members(info, depth).await? {
            match member {
                PropType::Union(inner) => flat.extend(inner),
                other => flat.push(other),
            }
        }
        // `boolean` is the union `false | true`.
        let has_both = flat.contains(&PropType::BoolLiteral(true)) && flat.contains(&PropType::BoolLiteral(false));
        let mut members: Vec<PropType> = Vec::new();
        for member in flat {
            let member = match member {
                PropType::BoolLiteral(_) if has_both => PropType::Boolean,
                other => other,
            };
            if !members.contains(&member) {
                members.push(member);
            }
        }
        Ok(if members.len() == 1 { members.swap_remove(0) } else { PropType::Union(members) })
    }

    async fn intersection(&self, info: &TypeInfo, depth: u8) -> Result<PropType, CallError> {
        // `string & {}` (the keep-the-literals trick) is a string.
        let mut members: Vec<PropType> = self
            .members(info, depth)
            .await?
            .into_iter()
            .filter(|member| !matches!(member, PropType::Object(fields) if fields.is_empty()))
            .collect();
        Ok(if members.len() == 1 { members.swap_remove(0) } else { PropType::Intersection(members) })
    }

    async fn object(&self, info: &TypeInfo, depth: u8) -> Result<PropType, CallError> {
        if info.is_tuple_type {
            let mut elements = Vec::new();
            for element in self.rpc.type_arguments(info.id).await? {
                elements.push(self.convert(&element, depth + 1).await?);
            }
            return Ok(PropType::Tuple(
                elements.into_iter().map(caliper_docgen_core::types::TupleElement::required).collect(),
            ));
        }
        if info.object_flags & flag::OBJECT_REFERENCE != 0 && self.rpc.is_array(info.id).await? {
            let arguments = self.rpc.type_arguments(info.id).await?;
            let Some(element) = arguments.first() else { return self.named(info).await };
            return Ok(PropType::Array(Box::new(self.convert(element, depth + 1).await?)));
        }
        let text = self.rpc.type_to_string(info.id).await?;
        match text.as_str() {
            "CSSProperties" | "React.CSSProperties" => return Ok(PropType::CssProperties),
            "JSX.Element" | "Element" => return Ok(PropType::ReactNode),
            _ if text.starts_with("ReactElement<") => return Ok(PropType::ReactNode),
            _ => {}
        }
        if text.starts_with('(') || text.starts_with('<') {
            return self.function(info, depth, text).await;
        }
        if text.starts_with('{') {
            return self.inline_object(info, depth, text).await;
        }
        Ok(PropType::Named { name: text.into(), args: Vec::new() })
    }

    async fn function(&self, info: &TypeInfo, depth: u8, text: String) -> Result<PropType, CallError> {
        let signatures = self.rpc.signatures(info.id).await?;
        let [signature] = signatures.as_slice() else {
            return Ok(PropType::Named { name: text.into(), args: Vec::new() });
        };
        let parameters = self.rpc.parameters(signature.id).await?;
        let references: Vec<_> = parameters.iter().map(|parameter| &parameter.reference).collect();
        let types = if references.is_empty() { Vec::new() } else { self.rpc.types_of_symbols(&references).await? };
        // The checker's parameter symbols say neither which parameter is optional nor which is the rest parameter;
        // its printed signature does (`percent?: number`, `...rest: string[]`), and its signature flags say whether
        // the last parameter is a rest parameter. A printed signature that does not parse into one entry per
        // parameter (a long one is cut short) leaves the markers off, as before.
        let markers = parameter_markers(&text).filter(|markers| markers.len() == parameters.len());
        let has_rest = signature.flags & flag::SIGNATURE_HAS_REST_PARAMETER != 0;
        let mut params = Vec::new();
        for (index, (parameter, ty)) in parameters.iter().zip(&types).enumerate() {
            let marker = markers.as_ref().and_then(|markers| markers.get(index)).copied().unwrap_or_default();
            let rest = marker.rest || (has_rest && index + 1 == parameters.len());
            let optional = marker.optional && !rest;
            let param_type = without_undefined(self.convert(ty, depth + 1).await?, optional);
            params.push(FunctionParam { name: Some(parameter.name.clone()), param_type, optional, rest });
        }
        let return_type = self.rpc.return_type(signature.id).await?;
        Ok(PropType::Function {
            type_params: Vec::new(),
            params,
            return_type: Box::new(self.convert(&return_type, depth + 1).await?),
        })
    }

    async fn inline_object(&self, info: &TypeInfo, depth: u8, text: String) -> Result<PropType, CallError> {
        let properties = self.rpc.properties(info.id).await?;
        if properties.is_empty() || properties.len() > MAX_INLINE_FIELDS {
            return Ok(PropType::Named { name: text.into(), args: Vec::new() });
        }
        let references: Vec<_> = properties.iter().map(|property| &property.reference).collect();
        let types = self.rpc.types_of_symbols(&references).await?;
        let mut fields = Vec::new();
        for (property, ty) in properties.iter().zip(&types) {
            let required = property.flags & flag::SYMBOL_OPTIONAL == 0;
            fields.push(ObjectField::new(
                property.name.clone(),
                without_undefined(self.convert(ty, depth + 1).await?, !required),
                required,
                String::new(),
            ));
        }
        Ok(PropType::Object(fields))
    }
}

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
    top_level_segments(list)?.into_iter().map(parameter_marker).collect()
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

/// The parts of a parameter list between its top-level commas.
fn top_level_segments(list: &[char]) -> Option<Vec<&[char]>> {
    let mut segments = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < list.len() {
        match list.get(i)? {
            '"' | '\'' | '`' => i = skip_string(list, i)?,
            '(' | '[' | '{' | '<' => i = matching_close(list, i)?,
            '=' if list.get(i + 1) == Some(&'>') => i += 1,
            ',' => {
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

/// An optional prop's type includes `undefined`; react-docgen-typescript drops it, as does the structural result.
pub(crate) fn without_undefined(prop_type: PropType, optional: bool) -> PropType {
    match prop_type {
        PropType::Union(members) if optional => {
            let mut kept: Vec<PropType> = members.into_iter().filter(|m| *m != PropType::Undefined).collect();
            if kept.len() == 1 { kept.swap_remove(0) } else { PropType::Union(kept) }
        }
        other => other,
    }
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
}
