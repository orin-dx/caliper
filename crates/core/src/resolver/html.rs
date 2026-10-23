//! HTML attribute inference helpers.

use crate::types::*;

pub(super) fn infer_html_attr_prop_type(attr_name: &str) -> PropType {
    match attr_name {
        "onClick" | "onKeyDown" | "onKeyUp" | "onFocus" | "onBlur" | "onChange" | "onInput" | "onSubmit"
        | "onReset" | "onLoad" | "onError" | "onPress" | "onPressStart" | "onPressEnd" | "onHoverStart"
        | "onHoverEnd" | "onFocusChange" | "onPressChange" => PropType::Function {
            type_params: vec![],
            params: vec![FunctionParam {
                name: Some("e".into()),
                param_type: PropType::Named { name: "Event".into(), args: vec![] },
            }],
            return_type: Box::new(PropType::Void),
        },
        "disabled" | "readOnly" | "required" | "checked" | "multiple" | "noValidate" | "autoFocus" | "fullWidth"
        | "loading" | "isDisabled" | "isReadOnly" | "isRequired" => PropType::Boolean,
        "tabIndex" | "rows" | "cols" | "maxLength" | "min" | "max" | "width" | "height" | "size" => PropType::Number,
        "style" => PropType::CssProperties,
        "children" => PropType::ReactNode,
        _ => PropType::String,
    }
}

pub(super) fn capitalize_element(element: &str) -> &'static str {
    match element {
        "button" => "Button",
        "input" => "Input",
        "a" => "Anchor",
        "textarea" => "Textarea",
        "select" => "Select",
        "form" => "Form",
        "label" => "Label",
        "img" => "Img",
        "video" => "Video",
        "audio" => "Audio",
        _ => "HTML",
    }
}
