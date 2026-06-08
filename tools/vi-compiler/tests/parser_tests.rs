use vi_compiler::{ast::*, compile_str};

fn parse_counter() -> ViFile {
    let src = include_str!("fixtures/counter.vi");
    compile_str(src).expect("counter.vi should parse without errors")
}

/// Unwrap a `Child::Element` — panics in tests if the child is `If` or `For`.
fn unwrap_elem(child: &Child) -> &Element {
    match child {
        Child::Element(e) => e,
        other => panic!("expected Child::Element, got {:?}", other),
    }
}

#[test]
fn counter_has_one_component() {
    let file = parse_counter();
    assert_eq!(file.components.len(), 1);
    assert_eq!(file.components[0].name, "Counter");
}

#[test]
fn counter_property_decl() {
    let file   = parse_counter();
    let comp   = &file.components[0];
    assert_eq!(comp.properties.len(), 1);
    let prop = &comp.properties[0];
    assert_eq!(prop.visibility, Some(Visibility::InOut));
    assert_eq!(prop.ty,   "int");
    assert_eq!(prop.name, "count");
    // Default should be Raw("0")
    match prop.default.as_ref().unwrap() {
        Expr::Raw(r) => assert_eq!(r.text.trim(), "0"),
    }
}

#[test]
fn counter_child_element() {
    let file = parse_counter();
    let comp = &file.components[0];
    assert_eq!(comp.children.len(), 1);
    assert_eq!(unwrap_elem(&comp.children[0]).name, "VerticalLayout");
}

#[test]
fn vertical_layout_bindings() {
    let file   = parse_counter();
    let vl     = unwrap_elem(&file.components[0].children[0]);
    let names:  Vec<_> = vl.bindings.iter().map(|b| b.property.as_str()).collect();
    assert!(names.contains(&"padding"), "padding binding missing");
    assert!(names.contains(&"spacing"), "spacing binding missing");
}

#[test]
fn vertical_layout_children() {
    let file = parse_counter();
    let vl   = unwrap_elem(&file.components[0].children[0]);
    let names: Vec<_> = vl.children.iter()
        .filter_map(|c| if let Child::Element(e) = c { Some(e.name.as_str()) } else { None })
        .collect();
    assert_eq!(names, vec!["Text", "Button"]);
}

#[test]
fn button_callback_binding() {
    let file   = parse_counter();
    let vl     = unwrap_elem(&file.components[0].children[0]);
    let button = vl.children.iter()
        .filter_map(|c| if let Child::Element(e) = c { Some(e) } else { None })
        .find(|e| e.name == "Button")
        .expect("Button missing");
    assert_eq!(button.callbacks.len(), 1);
    assert_eq!(button.callbacks[0].name, "clicked");
    assert!(!button.callbacks[0].body.is_empty(), "callback body should not be empty");
}

#[test]
fn text_binding_has_interpolation() {
    let file = parse_counter();
    let vl   = unwrap_elem(&file.components[0].children[0]);
    let text = vl.children.iter()
        .filter_map(|c| if let Child::Element(e) = c { Some(e) } else { None })
        .find(|e| e.name == "Text")
        .expect("Text missing");
    let tb   = text.bindings.iter().find(|b| b.property == "text").expect("text binding missing");
    match &tb.value {
        Expr::Raw(r) => assert!(r.text.contains("\\{count}"), "interpolation must be in raw text"),
    }
}

#[test]
fn error_on_unexpected_toplevel() {
    let result = compile_str("garbage");
    assert!(result.is_err(), "unexpected top-level token should be an error");
}

#[test]
fn empty_file_ok() {
    let file = compile_str("// just a comment").expect("empty file should parse");
    assert!(file.components.is_empty());
}
