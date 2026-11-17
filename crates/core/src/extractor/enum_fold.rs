//! The value of a TypeScript enum member's initializer, for the forms the compiler's constant evaluator folds without a
//! type checker: number and string literals, parentheses, the prefix operators `+ - ~`, the binary operators
//! `+ - * / % ** << >> >>> & | ^` over numbers (and `+` with a string operand), and a read of an earlier member of the
//! same declaration. Anything else is [`EnumValue::Unknown`]: the structural lane never guesses, and the checker lane
//! answers the rest (a reference to a `const`, a template with substitutions, a call).

use oxc_ast::ast::{BinaryOperator, Expression, TSEnumMemberName, UnaryOperator};
use oxc_str::JSStr;
use rustc_hash::FxHashMap;

use crate::types::EnumValue;

use super::js_text;

/// The members read so far in one declaration, by name.
pub(super) type EarlierMembers = FxHashMap<String, EnumValue>;

/// The name an enum member is read by, `None` for the computed template form no initializer can name.
pub(super) fn member_name(name: &TSEnumMemberName<'_>) -> Option<String> {
    match name {
        TSEnumMemberName::Identifier(id) => Some(id.name.as_str().to_owned()),
        TSEnumMemberName::String(s) => Some(js_text::lossy(s.value).into_owned()),
        TSEnumMemberName::ComputedString(s) => Some(js_text::lossy(s.value).into_owned()),
        TSEnumMemberName::ComputedTemplateString(_) => None,
    }
}

/// `expr` folded, in the scope of an enum called `enum_name` whose earlier members are `earlier`.
pub(super) fn fold(expr: &Expression<'_>, enum_name: &str, earlier: &EarlierMembers) -> EnumValue {
    match expr {
        Expression::NumericLiteral(n) => number(n.value),
        Expression::StringLiteral(s) => text(s.value),
        Expression::TemplateLiteral(t) => match (t.expressions.is_empty(), t.quasis.first()) {
            (true, Some(quasi)) => quasi.value.cooked.map_or(EnumValue::Unknown, text),
            _ => EnumValue::Unknown,
        },
        Expression::ParenthesizedExpression(p) => fold(&p.expression, enum_name, earlier),
        Expression::UnaryExpression(u) => match (u.operator, fold(&u.argument, enum_name, earlier)) {
            (UnaryOperator::UnaryPlus, EnumValue::Number(n)) => number(n),
            (UnaryOperator::UnaryNegation, EnumValue::Number(n)) => number(-n),
            (UnaryOperator::BitwiseNot, EnumValue::Number(n)) => number(f64::from(!to_int32(n))),
            _ => EnumValue::Unknown,
        },
        Expression::BinaryExpression(b) => {
            let left = fold(&b.left, enum_name, earlier);
            let right = fold(&b.right, enum_name, earlier);
            binary(b.operator, &left, &right)
        }
        Expression::Identifier(id) => earlier.get(id.name.as_str()).cloned().unwrap_or(EnumValue::Unknown),
        Expression::StaticMemberExpression(m) => match &m.object {
            Expression::Identifier(object) => {
                self_member(object.name.as_str(), m.property.name.as_str(), enum_name, earlier)
            }
            _ => EnumValue::Unknown,
        },
        Expression::ComputedMemberExpression(m) => match (&m.object, &m.expression) {
            (Expression::Identifier(object), Expression::StringLiteral(key)) => key
                .value
                .as_str()
                .map_or(EnumValue::Unknown, |key| self_member(object.name.as_str(), key, enum_name, earlier)),
            _ => EnumValue::Unknown,
        },
        Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::RegExpLiteral(_)
        | Expression::Super(_)
        | Expression::ArrayExpression(_)
        | Expression::ArrowFunctionExpression(_)
        | Expression::AssignmentExpression(_)
        | Expression::AwaitExpression(_)
        | Expression::CallExpression(_)
        | Expression::ChainExpression(_)
        | Expression::ClassExpression(_)
        | Expression::ConditionalExpression(_)
        | Expression::FunctionExpression(_)
        | Expression::ImportExpression(_)
        | Expression::LogicalExpression(_)
        | Expression::NewExpression(_)
        | Expression::ObjectExpression(_)
        | Expression::SequenceExpression(_)
        | Expression::TaggedTemplateExpression(_)
        | Expression::ThisExpression(_)
        | Expression::UpdateExpression(_)
        | Expression::YieldExpression(_)
        | Expression::PrivateInExpression(_)
        | Expression::PrivateFieldExpression(_)
        | Expression::ImportMeta(_)
        | Expression::NewTarget(_)
        | Expression::JSXElement(_)
        | Expression::JSXFragment(_)
        | Expression::TSAsExpression(_)
        | Expression::TSSatisfiesExpression(_)
        | Expression::TSTypeAssertion(_)
        | Expression::TSNonNullExpression(_)
        | Expression::TSInstantiationExpression(_)
        | Expression::V8IntrinsicExpression(_) => EnumValue::Unknown,
    }
}

/// The value of a string: its text when it is well-formed. A string holding a lone surrogate has no UTF-8 text, and
/// folding it as the replacement character would publish a value TypeScript does not compute (`"\uD83D" + "\uDE00"` is
/// one emoji there), so it is not a value here.
fn text(value: JSStr<'_>) -> EnumValue {
    value.as_str().map_or(EnumValue::Unknown, |text| EnumValue::String(text.to_owned()))
}

/// `object.member` where `object` is the enum itself and `member` is declared before it. A member named like the enum
/// shadows the enum inside its own declaration, so then no read names the enum.
fn self_member(object: &str, member: &str, enum_name: &str, earlier: &EarlierMembers) -> EnumValue {
    if object != enum_name || earlier.contains_key(enum_name) {
        return EnumValue::Unknown;
    }
    earlier.get(member).cloned().unwrap_or(EnumValue::Unknown)
}

/// A number that is finite, with `-0` as `0` (TypeScript prints both as `0`); anything else is not a value.
fn number(value: f64) -> EnumValue {
    if value.is_finite() { EnumValue::Number(if value == 0.0 { 0.0 } else { value }) } else { EnumValue::Unknown }
}

/// JavaScript's `ToInt32` of a finite number.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    (value.trunc().rem_euclid(4_294_967_296.0) as u32) as i32
}

#[allow(clippy::cast_sign_loss)]
fn to_uint32(value: f64) -> u32 {
    to_int32(value) as u32
}

fn binary(operator: BinaryOperator, left: &EnumValue, right: &EnumValue) -> EnumValue {
    match (operator, left, right) {
        (BinaryOperator::Addition, EnumValue::String(l), EnumValue::String(r)) => EnumValue::String(format!("{l}{r}")),
        (BinaryOperator::Addition, EnumValue::String(l), EnumValue::Number(r)) => {
            text_of(*r).map_or(EnumValue::Unknown, |r| EnumValue::String(format!("{l}{r}")))
        }
        (BinaryOperator::Addition, EnumValue::Number(l), EnumValue::String(r)) => {
            text_of(*l).map_or(EnumValue::Unknown, |l| EnumValue::String(format!("{l}{r}")))
        }
        (operator, EnumValue::Number(l), EnumValue::Number(r)) => numeric(operator, *l, *r),
        (
            BinaryOperator::Addition
            | BinaryOperator::Subtraction
            | BinaryOperator::Multiplication
            | BinaryOperator::Division
            | BinaryOperator::Remainder
            | BinaryOperator::Exponential
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight
            | BinaryOperator::ShiftRightZeroFill
            | BinaryOperator::BitwiseAnd
            | BinaryOperator::BitwiseOR
            | BinaryOperator::BitwiseXOR
            | BinaryOperator::Equality
            | BinaryOperator::Inequality
            | BinaryOperator::StrictEquality
            | BinaryOperator::StrictInequality
            | BinaryOperator::LessThan
            | BinaryOperator::LessEqualThan
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterEqualThan
            | BinaryOperator::In
            | BinaryOperator::Instanceof,
            _,
            _,
        ) => EnumValue::Unknown,
    }
}

fn numeric(operator: BinaryOperator, l: f64, r: f64) -> EnumValue {
    match operator {
        BinaryOperator::Addition => number(l + r),
        BinaryOperator::Subtraction => number(l - r),
        BinaryOperator::Multiplication => number(l * r),
        BinaryOperator::Division => number(l / r),
        BinaryOperator::Remainder => number(l % r),
        BinaryOperator::Exponential => number(l.powf(r)),
        BinaryOperator::ShiftLeft => number(f64::from(to_int32(l).wrapping_shl(to_uint32(r)))),
        BinaryOperator::ShiftRight => number(f64::from(to_int32(l).wrapping_shr(to_uint32(r)))),
        BinaryOperator::ShiftRightZeroFill => number(f64::from(to_uint32(l).wrapping_shr(to_uint32(r)))),
        BinaryOperator::BitwiseAnd => number(f64::from(to_int32(l) & to_int32(r))),
        BinaryOperator::BitwiseOR => number(f64::from(to_int32(l) | to_int32(r))),
        BinaryOperator::BitwiseXOR => number(f64::from(to_int32(l) ^ to_int32(r))),
        BinaryOperator::Equality
        | BinaryOperator::Inequality
        | BinaryOperator::StrictEquality
        | BinaryOperator::StrictInequality
        | BinaryOperator::LessThan
        | BinaryOperator::LessEqualThan
        | BinaryOperator::GreaterThan
        | BinaryOperator::GreaterEqualThan
        | BinaryOperator::In
        | BinaryOperator::Instanceof => EnumValue::Unknown,
    }
}

/// The text JavaScript gives a number in a `+` with a string, for an integer small enough that no exponent form comes
/// into it; other numbers are not folded.
#[allow(clippy::cast_possible_truncation)]
fn text_of(value: f64) -> Option<String> {
    (value.fract() == 0.0 && value.abs() < 1e15).then(|| (value as i64).to_string())
}
