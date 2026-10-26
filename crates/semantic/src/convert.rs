//! A checker type reduced to the existing [`PropType`], where that is lossless; otherwise the checker's own text as a
//! `Named` type. The output schema is unchanged: nothing here adds a variant.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use caliper_docgen_core::types::{FunctionParam, ObjectField, PropType};
use rustc_hash::FxHashMap;

use crate::rpc::{flag, CallError, Rpc, SymbolInfo, TypeInfo};

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
            return Ok(PropType::Tuple(elements));
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
        let mut params = Vec::new();
        for (parameter, ty) in parameters.iter().zip(&types) {
            params.push(FunctionParam::new(Some(parameter.name.clone()), self.convert(ty, depth + 1).await?));
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
            fields.push(ObjectField {
                name: property.name.clone(),
                prop_type: without_undefined(self.convert(ty, depth + 1).await?, !required),
                required,
                description: String::new(),
            });
        }
        Ok(PropType::Object(fields))
    }
}

/// An optional prop's type includes `undefined`; react-docgen-typescript drops it, as does the structural result.
pub(crate) fn without_undefined(prop_type: PropType, optional: bool) -> PropType {
    match prop_type {
        PropType::Union(members) if optional => {
            let mut kept: Vec<PropType> = members.into_iter().filter(|m| *m != PropType::Undefined).collect();
            if kept.len() == 1 {
                kept.swap_remove(0)
            } else {
                PropType::Union(kept)
            }
        }
        other => other,
    }
}
