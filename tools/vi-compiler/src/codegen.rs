//! Rust codegen — walks a `ViFile` AST and emits Rust source using the P02 Layer 2 API.
//!
//! Generated code targets a `no_std + alloc` environment (ViCell Cell context).
//! Output uses `alloc::format!`, `alloc::vec![]`, `alloc::boxed::Box` explicitly.
//!
//! Each component is wrapped in an isolated `mod __vi_generated_<Name>` to prevent
//! `use` import conflicts when multiple `vi_design!` / `include!()` calls appear in
//! the same scope. `pub use __vi_generated_<Name>::*` re-exports the struct.
//!
//! # Usage
//!
//! ```ignore
//! let file = vi_compiler::compile_str(src)?;
//! let mut gen = CodeGen::new();
//! let rust_src = gen.generate(&file);
//! ```

use std::prelude::v1::*;
use crate::ast::{Binding, CallbackBinding, Child, Component, Element, Expr, ViFile};
use crate::eval::{
    compile_expr, eval_binding, eval_callback, eval_property,
    AugOp, ExprCtx, InterpolPart, TypedExpr,
};

// ─── Element → Widget mapping ────────────────────────────────────────────────

/// Maps a `.vi` element name to `(Rust type, import path segment)`.
///
/// Returns `None` for unknown elements — callers should emit `compile_error!`.
fn map_element(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        // Layout
        "VerticalLayout" | "VBox" | "Column" => Some(("Column",      "column")),
        "HorizontalLayout" | "HBox" | "Row"  => Some(("Row",         "row")),
        "FlexBox" | "HFlex" | "VFlex"        => Some(("FlexBox",     "flex_box")),
        // Text
        "Text" | "Label"                     => Some(("Label",       "label")),
        // Interactive
        "Button"                             => Some(("Button",      "button")),
        "Slider"                             => Some(("Slider",      "slider")),
        "CheckBox" | "Checkbox"              => Some(("CheckBox",    "checkbox")),
        // Display
        "ProgressBar" | "Progress"           => Some(("ProgressBar", "progress_bar")),
        "Image"                              => Some(("Image",       "image")),
        // Input
        "TextInput" | "TextEdit"             => Some(("TextEdit",    "text_edit")),
        // Container
        "TouchArea"                          => Some(("TouchArea",   "touch_area")),
        "ListView" | "List"                  => Some(("ListView",    "list_view")),
        "ScrollArea" | "ScrollView"          => Some(("ScrollArea",  "scroll_area")),
        // Overlay widgets — constructed imperatively; codegen emits a placeholder.
        "Dialog"                             => Some(("Dialog",      "dialog")),
        "DropDown" | "Dropdown"              => Some(("DropDown",    "dropdown")),
        _ => None,
    }
}

/// Describes how a widget's Rust constructor is called.
// Prepared for Phase-level refactor; not yet wired into gen_element dispatch.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
enum CtorStyle {
    /// `Container::new(children_vec)` — layout containers (Column, Row).
    Container,
    /// `Widget::new(child_box)` — single-child containers (TouchArea, ScrollArea).
    ChildBox,
    /// `Widget::new(signal)` — first property binding becomes the constructor arg.
    ///
    /// Used by Label, ProgressBar, Slider, CheckBox, TextEdit.
    SignalFirst,
    /// `Button::new(label_str, callback)` — special: string label + closure.
    SignalCallback,
    /// `ListView::new(items_signal)` — `Signal<Vec<String>>` constructor.
    ItemsSignal,
    /// `Widget::new()` — no mandatory constructor argument (Image placeholder, etc.).
    NoArg,
}

/// Returns the constructor style for a Rust widget type produced by `map_element`.
#[allow(dead_code)]
fn widget_ctor_style(rust_type: &str) -> CtorStyle {
    match rust_type {
        "Column" | "Row" | "FlexBox" => CtorStyle::Container,
        "TouchArea" | "ScrollArea"   => CtorStyle::ChildBox,
        "Label"                      => CtorStyle::SignalFirst,
        "Button"                     => CtorStyle::SignalCallback,
        "ProgressBar" | "Slider"     => CtorStyle::SignalFirst,
        // CheckBox::new(checked: Signal<bool>) — confirmed from source
        "CheckBox"                   => CtorStyle::SignalFirst,
        // TextEdit::new(text: Signal<String>) — confirmed from source
        "TextEdit"                   => CtorStyle::SignalFirst,
        "ListView"                   => CtorStyle::ItemsSignal,
        // Dialog / DropDown require runtime queue — emit imperative placeholder
        "Dialog" | "DropDown"        => CtorStyle::NoArg,
        // Image::new(data, width, height) — too complex; emit empty placeholder
        _                            => CtorStyle::NoArg,
    }
}

/// Emit a builder-method chain call for a known widget property.
///
/// Returns `Some(".method(expr)")` for known properties that map to a builder
/// method, `None` for properties consumed by the constructor or unknown ones.
/// Unknown properties are silently skipped (a future P10 pass will warn).
fn emit_builder_call(prop: &str, expr: &str) -> Option<String> {
    match prop {
        // These are consumed by the constructor — not builder calls.
        "text" | "value" | "items" | "checked" => None,
        // Known builder methods.
        "color"       => Some(format!(".color({})", expr)),
        "item_height" => Some(format!(".item_height({}f32)", expr)),
        // padding / spacing handled by layout containers directly.
        "padding" | "spacing" => None,
        // Unknown property — silently skip (no builder method known).
        _ => None,
    }
}

/// Maps a `.vi` property type string to a Rust type string.
fn rust_type(ty: &str) -> &str {
    match ty {
        "int"    => "i32",
        "float"  => "f32",
        "string" => "String",
        "bool"   => "bool",
        "color"  => "viui::canvas::Color",
        "length" => "f32",
        _        => "i32",
    }
}

/// Default Rust expression for initialising a `Signal<T>` with a given type.
fn default_signal_init(ty: &str) -> &str {
    match ty {
        "int"    => "Signal::new(0i32)",
        "float"  => "Signal::new(0.0f32)",
        "string" => "Signal::new(String::new())",
        "bool"   => "Signal::new(false)",
        "color"  => "Signal::new(viui::canvas::Color::TRANSPARENT)",
        "length" => "Signal::new(0.0f32)",
        _        => "Signal::new(0i32)",
    }
}

// ─── CodeGen ─────────────────────────────────────────────────────────────────

/// Per-component state carried during emission.
struct CompState {
    clone_counter:  usize,
    sub_counter:    usize,
    widget_counter: usize,
}

impl CompState {
    fn new() -> Self {
        Self { clone_counter: 0, sub_counter: 0, widget_counter: 0 }
    }

    fn next_clone(&mut self, base: &str) -> String {
        let n = self.clone_counter;
        self.clone_counter += 1;
        format!("_{}_c{}", base, n)
    }

    fn next_sub(&mut self) -> String {
        let n = self.sub_counter;
        self.sub_counter += 1;
        format!("_sub_text_{}", n)
    }

    fn next_widget(&mut self, ty: &str) -> String {
        let n = self.widget_counter;
        self.widget_counter += 1;
        format!("w_{}_{}", ty.to_lowercase(), n)
    }
}

/// Code generator: produces Rust source from a `ViFile` AST.
pub struct CodeGen;

impl Default for CodeGen {
    fn default() -> Self { Self }
}

impl CodeGen {
    pub fn new() -> Self { Self }

    /// Generate Rust source for all components in `file`.
    ///
    /// Each component is placed inside an isolated module (`__vi_generated_<Name>`)
    /// that holds its own `use` imports, then re-exported with `pub use ...::*`.
    /// This prevents duplicate-import errors when multiple `vi_design!` or
    /// `include!()` calls appear in the same module scope.
    pub fn generate(&mut self, file: &ViFile) -> String {
        let mut out = String::new();
        out.push_str("// Generated by vi-compiler \u{2014} DO NOT EDIT\n");

        for comp in &file.components {
            let comp_src = self.gen_component(comp);
            let mod_name = format!("__vi_generated_{}", comp.name);

            out.push_str("#[allow(non_snake_case, unused_imports)]\n");
            out.push_str(&format!("mod {} {{\n", mod_name));
            // Imports scoped to this module — no leakage into the caller's namespace.
            out.push_str("    use alloc::string::String;\n");
            out.push_str("    use viui::signal::{Signal, SubscriptionHandle};\n");
            out.push_str("    use viui::node::ViNode;\n");
            out.push_str("    use viui::node_widgets::label::Label;\n");
            out.push_str("    use viui::node_widgets::button::Button;\n");
            out.push_str("    use viui::node_widgets::column::Column;\n");
            out.push_str("    use viui::node_widgets::row::Row;\n");
            out.push_str("    use viui::node_widgets::progress_bar::ProgressBar;\n");
            out.push_str("    use viui::node_widgets::slider::Slider;\n");
            out.push_str("    use viui::node_widgets::checkbox::CheckBox;\n");
            out.push_str("    use viui::node_widgets::list_view::ListView;\n");
            out.push_str("    use viui::node_widgets::text_edit::TextEdit;\n");
            out.push_str("    use viui::node_widgets::touch_area::TouchArea;\n");
            out.push_str("    use viui::node_widgets::scroll_area::ScrollArea;\n");
            out.push_str("    use viui::node_widgets::image::Image;\n");
            out.push_str("    use viui::node_widgets::flex_box::FlexBox;\n");
            out.push_str("    use viui::node_widgets::card::Card;\n");
            out.push_str("    use viui::node_widgets::divider::Divider;\n");
            out.push_str("    use viui::node_widgets::space::Space;\n");
            out.push_str("    use viui::node_widgets::dialog::Dialog;\n");
            out.push_str("    use viui::node_widgets::dropdown::DropDown;\n");
            out.push_str("    use viui::canvas::Color;\n\n");

            // Indent each line of component source by 4 spaces.
            for line in comp_src.lines() {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str("    ");
                    out.push_str(line);
                    out.push('\n');
                }
            }

            out.push_str("}\n");
            out.push_str(&format!("pub use {}::*;\n\n", mod_name));
        }
        out
    }

    fn gen_component(&mut self, comp: &Component) -> String {
        let mut st = CompState::new();

        let mut sub_fields:  Vec<String> = Vec::new();
        let mut build_stmts: Vec<String> = Vec::new();

        // 1. Emit Signal declarations for in/out/in-out properties.
        for prop in &comp.properties {
            let init = match &prop.default {
                Some(Expr::Raw(r)) => {
                    typed_expr_to_rust(&eval_property(&r.text, &prop.ty), &prop.ty)
                }
                Some(other) => {
                    // Typed AST node — convert to TypedExpr for consistent Signal::new wrapping.
                    typed_expr_to_rust(&expr_to_typed(other, &prop.ty), &prop.ty)
                }
                None => default_signal_init(&prop.ty).to_string(),
            };
            build_stmts.push(format!("        let {} = {};", prop.name, init));
        }
        if !comp.properties.is_empty() { build_stmts.push(String::new()); }

        // 2. Walk element tree.
        let root_widget = if let Some(root_child) = comp.children.first() {
            let prop_name_list: Vec<&str> = comp.properties.iter()
                .map(|p| p.name.as_str())
                .collect();
            let (stmts, subs, var_name) =
                self.gen_child(root_child, &prop_name_list, &mut st);
            for s in stmts  { build_stmts.push(s); }
            for sub in &subs { sub_fields.push(sub.clone()); }
            var_name
        } else {
            "Column::new(alloc::vec![])".to_string()
        };

        // 3. Root widget type for return signature.
        let root_ty = comp.children.first()
            .and_then(|c| {
                if let Child::Element(e) = c {
                    map_element(&e.name).map(|(t, _)| t)
                } else {
                    // if/for at root → wrap in Column
                    Some("Column")
                }
            })
            .unwrap_or("Column");

        // ── Emit struct ───────────────────────────────────────────────────────
        let mut out = String::new();
        out.push_str(&format!("/// Generated component: {}\n", comp.name));
        out.push_str(&format!("pub struct {} {{\n", comp.name));
        for prop in &comp.properties {
            let vis = prop.visibility.as_ref().map(|v| format!("{:?}", v)).unwrap_or_default();
            if vis != "Private" {
                out.push_str(&format!("    pub {}: Signal<{}>,\n", prop.name, rust_type(&prop.ty)));
            }
        }
        for sub_name in &sub_fields {
            out.push_str(&format!("    {}: SubscriptionHandle,\n", sub_name));
        }
        out.push_str("}\n\n");

        // ── Emit impl ─────────────────────────────────────────────────────────
        out.push_str(&format!("impl {} {{\n", comp.name));
        out.push_str(&format!("    pub fn build() -> (Self, {}) {{\n", root_ty));
        for s in &build_stmts { out.push_str(s); out.push('\n'); }

        let fields_str = comp.properties.iter()
            .map(|p| p.name.clone())
            .chain(sub_fields.iter().cloned())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("        let state = Self {{ {} }};\n", fields_str));
        out.push_str(&format!("        (state, {})\n", root_widget));
        out.push_str("    }\n");
        out.push_str("}\n");
        out
    }

    // ── gen_child ────────────────────────────────────────────────────────────

    /// Dispatch to the appropriate generator based on `Child` variant.
    ///
    /// Returns `(statements, subscription_handle_names, widget_variable_name)`.
    fn gen_child(
        &mut self,
        child: &Child,
        prop_names: &[&str],
        st: &mut CompState,
    ) -> (Vec<String>, Vec<String>, String) {
        match child {
            Child::Element(e) => self.gen_element(e, prop_names, st),

            Child::If { cond, body, .. } => {
                let mut stmts: Vec<String> = Vec::new();
                let mut subs:  Vec<String> = Vec::new();
                let var = st.next_widget("if");

                // Generate body children
                let mut child_vars: Vec<String> = Vec::new();
                for c in body {
                    let (cs, ss, v) = self.gen_child(c, prop_names, st);
                    stmts.extend(cs);
                    subs.extend(ss);
                    child_vars.push(v);
                }

                let items_str: String = child_vars.iter()
                    .map(|v| format!(
                        "alloc::boxed::Box::new({}) as alloc::boxed::Box<dyn ViNode>",
                        v
                    ))
                    .collect::<Vec<_>>()
                    .join(", ");

                // Desugar `self.X` → `*self.X.get()` in the raw condition expression.
                let desugared = desugar_prop_refs(cond);
                stmts.push(format!(
                    "        let {}_items = if {} {{ alloc::vec![{}] }} else {{ alloc::vec![] }};",
                    var, desugared, items_str
                ));
                stmts.push(format!(
                    "        let {} = Column::new({}_items);",
                    var, var
                ));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            Child::For { var: loop_var, iter, body, .. } => {
                let mut stmts: Vec<String> = Vec::new();
                let subs: Vec<String> = Vec::new();
                let container = st.next_widget("for");

                let desugared_iter = desugar_prop_refs(iter);

                // G1: single-item body supported. Each iteration produces one Box<dyn ViNode>.
                let inner_item_expr = if let Some(Child::Element(e)) = body.first() {
                    match e.name.as_str() {
                        "Text" | "Label" => {
                            let text_val = e.bindings.iter()
                                .find(|b| b.property == "text")
                                .map(|b| expr_as_raw_text(&b.value))
                                .unwrap_or_else(|| loop_var.clone());
                            format!(
                                "alloc::boxed::Box::new(Label::new(Signal::new(({}).to_string()))) \
                                 as alloc::boxed::Box<dyn ViNode>",
                                desugar_prop_refs(&text_val)
                            )
                        }
                        _ => {
                            "alloc::boxed::Box::new(Column::new(alloc::vec![])) \
                             as alloc::boxed::Box<dyn ViNode>".to_string()
                        }
                    }
                } else {
                    "alloc::boxed::Box::new(Column::new(alloc::vec![])) \
                     as alloc::boxed::Box<dyn ViNode>".to_string()
                };

                // `enumerate()` is present for parity with future indexed templates.
                stmts.push(format!(
                    "        let {container}_items: alloc::vec::Vec<alloc::boxed::Box<dyn ViNode>> = \
                     ({iter}).iter().enumerate().map(|(_{idx}_idx, {lv})| {{ {inner} }}).collect();",
                    container = container,
                    iter      = desugared_iter,
                    idx       = loop_var,
                    lv        = loop_var,
                    inner     = inner_item_expr,
                ));
                stmts.push(format!(
                    "        let {} = Column::new({}_items);",
                    container, container
                ));
                stmts.push(String::new());
                (stmts, subs, container)
            }
        }
    }

    // ── gen_element ──────────────────────────────────────────────────────────

    /// Recursively generate code for one concrete element.
    ///
    /// Returns `(statements, subscription_handle_names, widget_variable_name)`.
    fn gen_element(
        &mut self,
        elem: &Element,
        prop_names: &[&str],
        st: &mut CompState,
    ) -> (Vec<String>, Vec<String>, String) {
        let mut stmts: Vec<String> = Vec::new();
        let mut subs:  Vec<String> = Vec::new();

        match elem.name.as_str() {
            "VerticalLayout" | "VBox" | "Column" | "HorizontalLayout" | "HBox" | "Row" => {
                let is_vertical = matches!(elem.name.as_str(),
                    "VerticalLayout" | "VBox" | "Column");
                let container_ty = if is_vertical { "Column" } else { "Row" };

                let mut child_vars: Vec<String> = Vec::new();
                for child in &elem.children {
                    let (child_stmts, child_subs, child_var) =
                        self.gen_child(child, prop_names, st);
                    stmts.extend(child_stmts);
                    subs.extend(child_subs);
                    child_vars.push(child_var);
                }

                let vec_items: Vec<String> = child_vars.iter()
                    .map(|v| format!(
                        "alloc::boxed::Box::new({}) as alloc::boxed::Box<dyn ViNode>", v
                    ))
                    .collect();
                let vec_expr = format!("alloc::vec![{}]", vec_items.join(", "));
                let var = st.next_widget(container_ty);

                let padding = find_binding_f32(&elem.bindings, "padding");
                let spacing = find_binding_f32(&elem.bindings, "spacing");

                let mut init = format!("{}::new({})", container_ty, vec_expr);
                if let Some(p) = padding { init = format!("{}.with_padding({}f32)", init, p); }
                if let Some(s) = spacing { init = format!("{}.with_spacing({}f32)", init, s); }

                stmts.push(format!("        let {} = {};", var, init));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            "Text" | "Label" => {
                let var = st.next_widget("Label");

                let text_expr = if let Some(b) = elem.bindings.iter().find(|b| b.property == "text") {
                    match &b.value {
                        Expr::Raw(r) => eval_binding(&r.text, "text", "Text"),
                        Expr::Literal(crate::ast::Literal::Str(s)) => {
                            // Plain string literal — re-use eval's string parsing.
                            eval_binding(&format!("{:?}", s), "text", "Text")
                        }
                        Expr::Interpolated(parts) => {
                            // Convert ast::InterpPart → eval::InterpolPart for the
                            // existing .into_parts() codegen path.
                            let eval_parts: Vec<InterpolPart> = parts.iter().map(|p| {
                                match p {
                                    crate::ast::InterpPart::Lit(s) => InterpolPart::Literal(s.clone()),
                                    crate::ast::InterpPart::Expr(e) => {
                                        // Extract the variable name from the expression.
                                        let var = match e.as_ref() {
                                            Expr::Ident(n) | Expr::SelfProp(n) => n.clone(),
                                            other_e => compile_expr(other_e, ExprCtx::BuildFn),
                                        };
                                        InterpolPart::Var(var)
                                    }
                                }
                            }).collect();
                            TypedExpr::Interpolated(eval_parts)
                        }
                        Expr::SelfProp(prop) => {
                            // Reactive text binding: `text: self.X` → reactive map over the
                            // source signal.  Route through the Interpolated path so the
                            // existing `.map().into_parts()` codegen fires.
                            TypedExpr::Interpolated(vec![InterpolPart::Var(prop.clone())])
                        }
                        Expr::Ident(name) => {
                            // Bare identifier — reactive map over the local signal variable.
                            TypedExpr::Interpolated(vec![InterpolPart::Var(name.clone())])
                        }
                        other => {
                            let rs = compile_expr(other, ExprCtx::BuildFn);
                            TypedExpr::Ident(rs)
                        }
                    }
                } else {
                    TypedExpr::StringLit(String::new())
                };

                let text_sig_var = match &text_expr {
                    TypedExpr::Interpolated(parts) => {
                        let var_names: Vec<&str> = parts.iter()
                            .filter_map(|p| if let InterpolPart::Var(v) = p { Some(v.as_str()) } else { None })
                            .collect();
                        let src_sig = var_names.first().copied().unwrap_or("count");
                        let fmt_str  = build_format_string(parts);
                        let fmt_args = build_format_args(parts);
                        let sub_name = st.next_sub();
                        let text_sig = format!("text_sig_{}", st.sub_counter - 1);
                        stmts.push(format!(
                            "        let ({}, {}) = {}.map(|n| alloc::format!(\"{}\", {})).into_parts();",
                            text_sig, sub_name, src_sig, fmt_str, fmt_args
                        ));
                        stmts.push(String::new());
                        subs.push(sub_name);
                        text_sig
                    }
                    TypedExpr::StringLit(s) => {
                        // Use String::from() — no `ToString` trait import needed in no_std.
                        let sig_var = format!("text_sig_{}", st.sub_counter);
                        stmts.push(format!(
                            "        let {} = Signal::new(String::from(\"{}\"));",
                            sig_var, escape_str(s)
                        ));
                        sig_var
                    }
                    other => {
                        let sig_var = format!("text_sig_{}", st.sub_counter);
                        stmts.push(format!(
                            "        let {} = Signal::new(alloc::format!(\"{{}}\", {}));",
                            sig_var, typed_expr_raw(other)
                        ));
                        sig_var
                    }
                };

                let color_expr = elem.bindings.iter()
                    .find(|b| b.property == "color")
                    .map(|b| match &b.value {
                        Expr::Raw(r) => color_typed_to_rust(&eval_binding(&r.text, "color", "Text")),
                        other => compile_expr(other, ExprCtx::BuildFn),
                    });

                let mut init = format!("Label::new({})", text_sig_var);
                if let Some(ce) = color_expr {
                    init = format!("{}.with_color({})", init, ce);
                }

                stmts.push(format!("        let {} = {};", var, init));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            "Button" => {
                let var = st.next_widget("Button");

                let label_str = elem.bindings.iter()
                    .find(|b| b.property == "text")
                    .map(|b| match &b.value {
                        Expr::Raw(r) => match eval_binding(&r.text, "text", "Button") {
                            TypedExpr::StringLit(s) => s,
                            other => typed_expr_raw(&other),
                        },
                        Expr::Literal(crate::ast::Literal::Str(s)) => s.clone(),
                        other => compile_expr(other, ExprCtx::BuildFn),
                    })
                    .unwrap_or_default();

                let callback_body = gen_callback_body(&elem.callbacks, prop_names, st, &mut stmts);
                let init = format!("Button::new(\"{}\", {})", escape_str(&label_str), callback_body);
                stmts.push(format!("        let {} = {};", var, init));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── ProgressBar / Slider / CheckBox / TextEdit ───────────────────
            // All use SignalFirst: Widget::new(signal) where the signal comes
            // from the first relevant binding ("value", "checked", "text").
            "ProgressBar" | "Progress"
            | "Slider"
            | "CheckBox" | "Checkbox"
            | "TextInput" | "TextEdit" => {
                let (rust_ty, _) = map_element(elem.name.as_str())
                    .expect("map_element covers these arms");
                let var = st.next_widget(rust_ty);

                // Determine which binding name is the signal-first argument.
                let signal_prop = match rust_ty {
                    "CheckBox"  => "checked",
                    "TextEdit"  => "text",
                    _           => "value",   // ProgressBar, Slider
                };

                let sig_var = find_signal_binding(
                    &elem.bindings, signal_prop, rust_ty, &mut stmts, st,
                );

                // Remaining bindings become builder calls.
                let mut init = format!("{}::new({})", rust_ty, sig_var);
                for b in &elem.bindings {
                    if b.property == signal_prop { continue; }
                    let expr_str = expr_as_raw_text(&b.value);
                    if let Some(chain) = emit_builder_call(&b.property, &expr_str) {
                        init.push_str(&chain);
                    }
                }

                stmts.push(format!("        let {} = {};", var, init));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── ListView / List ───────────────────────────────────────────────
            // ListView::new(items: Signal<Vec<String>>)
            "ListView" | "List" => {
                let var = st.next_widget("ListView");

                let sig_var = find_signal_binding(
                    &elem.bindings, "items", "ListView", &mut stmts, st,
                );

                let mut init = format!("ListView::new({})", sig_var);
                for b in &elem.bindings {
                    if b.property == "items" { continue; }
                    let expr_str = expr_as_raw_text(&b.value);
                    if let Some(chain) = emit_builder_call(&b.property, &expr_str) {
                        init.push_str(&chain);
                    }
                }

                stmts.push(format!("        let {} = {};", var, init));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── TouchArea / ScrollArea ─────────────────────────────────────────
            // These wrap a single child; children vec is flattened into the first child.
            "TouchArea" | "ScrollArea" | "ScrollView" => {
                let (rust_ty, _) = map_element(elem.name.as_str())
                    .expect("map_element covers these arms");
                let var = st.next_widget(rust_ty);

                // Emit first child (or empty Column as placeholder).
                let child_expr = if let Some(first_child) = elem.children.first() {
                    let (child_stmts, child_subs, child_var) =
                        self.gen_child(first_child, prop_names, st);
                    stmts.extend(child_stmts);
                    subs.extend(child_subs);
                    format!(
                        "alloc::boxed::Box::new({}) as alloc::boxed::Box<dyn ViNode>",
                        child_var
                    )
                } else {
                    "alloc::boxed::Box::new(Column::new(alloc::vec![])) \
                     as alloc::boxed::Box<dyn ViNode>".to_string()
                };

                stmts.push(format!("        let {} = {}::new({});", var, rust_ty, child_expr));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── Image ─────────────────────────────────────────────────────────
            // Image::new(data, width, height) is too complex to auto-map from .vi
            // bindings alone. Emit a zero-size placeholder with a clarifying comment.
            "Image" => {
                let var = st.next_widget("Image");
                stmts.push(format!(
                    "        // Image: set data/width/height on {} after build()", var
                ));
                stmts.push(format!(
                    "        let {} = Image::new(\
                     Signal::new(None), 0u32, 0u32);",
                    var
                ));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── FlexBox / HFlex / VFlex ───────────────────────────────────────
            // Builder pattern: FlexBox::row()|column() + .child() chain.
            "FlexBox" | "HFlex" | "VFlex" => {
                let is_row = !matches!(elem.name.as_str(), "VFlex");
                let ctor = if is_row { "FlexBox::row()" } else { "FlexBox::column()" };
                let var = st.next_widget("FlexBox");

                let mut child_vars: Vec<String> = Vec::new();
                for child in &elem.children {
                    let (child_stmts, child_subs, child_var) =
                        self.gen_child(child, prop_names, st);
                    stmts.extend(child_stmts);
                    subs.extend(child_subs);
                    child_vars.push(child_var);
                }

                let chain: String = child_vars.iter()
                    .map(|v| format!(".child({})", v))
                    .collect::<Vec<_>>()
                    .join("");
                let gap     = find_binding_f32(&elem.bindings, "gap");
                let padding = find_binding_f32(&elem.bindings, "padding");
                let mut init = format!("{}{}", ctor, chain);
                if let Some(g) = gap     { init = format!("{}.gap({}f32)", init, g); }
                if let Some(p) = padding { init = format!("{}.padding({}f32)", init, p); }

                stmts.push(format!("        let {} = {};", var, init));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── Card ──────────────────────────────────────────────────────────
            // Card::new(Box<dyn ViNode>) — wraps a single child.
            "Card" => {
                let var = st.next_widget("Card");
                let child_expr = if let Some(first) = elem.children.first() {
                    let (cs, ss, cv) = self.gen_child(first, prop_names, st);
                    stmts.extend(cs);
                    subs.extend(ss);
                    format!("alloc::boxed::Box::new({}) as alloc::boxed::Box<dyn ViNode>", cv)
                } else {
                    "alloc::boxed::Box::new(Column::new(alloc::vec![])) \
                     as alloc::boxed::Box<dyn ViNode>".to_string()
                };
                stmts.push(format!("        let {} = Card::new({});", var, child_expr));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── Divider ───────────────────────────────────────────────────────
            "Divider" => {
                let var = st.next_widget("Divider");
                stmts.push(format!("        let {} = Divider::horizontal();", var));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── Space ─────────────────────────────────────────────────────────
            // Space::new(width, height) — flexible spacer.
            "Space" => {
                let var = st.next_widget("Space");
                let w = find_binding_f32(&elem.bindings, "width").unwrap_or(0.0);
                let h = find_binding_f32(&elem.bindings, "height").unwrap_or(0.0);
                stmts.push(format!("        let {} = Space::new({}f32, {}f32);", var, w, h));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── Dialog ────────────────────────────────────────────────────────
            // Dialog requires a runtime `OverlayActionQueue` that cannot be
            // inferred from .vi bindings alone. Emit an alert() placeholder that
            // compiles; the caller replaces it with the appropriate constructor.
            "Dialog" => {
                let var = st.next_widget("Dialog");
                let title_str = elem.bindings.iter()
                    .find(|b| b.property == "title")
                    .map(|b| expr_as_raw_text(&b.value))
                    .unwrap_or_else(|| "Dialog".to_string());
                let msg_str = elem.bindings.iter()
                    .find(|b| b.property == "message")
                    .map(|b| expr_as_raw_text(&b.value))
                    .unwrap_or_default();
                stmts.push(format!(
                    "        // Dialog: replace queue placeholder with app.action_queue()"
                ));
                stmts.push(format!(
                    "        let {var} = Dialog::alert(\"{title}\", \"{msg}\", \
                     viui::overlay::new_action_queue(), || {{}});",
                    var   = var,
                    title = escape_str(&title_str),
                    msg   = escape_str(&msg_str),
                ));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── DropDown / Dropdown ───────────────────────────────────────────
            // DropDown::new(selected, items, queue).
            // Emit a placeholder with an empty items list; the caller fills it in.
            "DropDown" | "Dropdown" => {
                let var = st.next_widget("DropDown");
                let sig_var = find_signal_binding(
                    &elem.bindings, "selected", "DropDown", &mut stmts, st,
                );
                stmts.push(format!(
                    "        // DropDown: replace queue placeholder with app.action_queue()"
                ));
                stmts.push(format!(
                    "        let {var} = DropDown::new({sig}, alloc::vec![], \
                     viui::overlay::new_action_queue());",
                    var = var,
                    sig = sig_var,
                ));
                stmts.push(String::new());
                (stmts, subs, var)
            }

            // ── Unknown widget ────────────────────────────────────────────────
            // Emit a compile_error! so the .vi source fails at Rust compile time
            // with a clear diagnostic instead of silently building a broken UI.
            unknown => {
                let var = format!("_unknown_{}", unknown.to_lowercase());
                stmts.push(format!(
                    "        compile_error!(\"vi-compiler: unknown widget '{}' at line {}\");",
                    unknown, elem.span.line
                ));
                // Emit a fallback placeholder so the rest of codegen can proceed
                // (the compile_error! above will halt compilation anyway).
                stmts.push(format!("        let {} = Column::new(alloc::vec![]);", var));
                stmts.push(String::new());
                (stmts, subs, var)
            }
        }
    }
}

// ─── Emit helpers ─────────────────────────────────────────────────────────────

fn gen_callback_body(
    callbacks: &[CallbackBinding],
    _prop_names: &[&str],
    st: &mut CompState,
    stmts: &mut Vec<String>,
) -> String {
    let clicked = callbacks.iter().find(|c| c.name == "clicked");
    let Some(cb) = clicked else {
        return "|| {}".to_string();
    };

    let typed = eval_callback(&cb.body);
    match typed {
        TypedExpr::AugAssign { name, op, rhs } => {
            let clone_var = st.next_clone(&name);
            stmts.push(format!("        let {} = {}.clone();", clone_var, name));
            let op_str = aug_op_to_method_body(&op, &rhs);
            format!("move || {{ {}.update(|n| {}); }}", clone_var, op_str)
        }
        _ => format!("|| {{ {} }}", cb.body.trim()),
    }
}

fn aug_op_to_method_body(op: &AugOp, rhs: &TypedExpr) -> String {
    let rhs_str = typed_expr_raw(rhs);
    match op {
        AugOp::Add => format!("*n += {}", rhs_str),
        AugOp::Sub => format!("*n -= {}", rhs_str),
        AugOp::Mul => format!("*n *= {}", rhs_str),
        AugOp::Div => format!("*n /= {}", rhs_str),
    }
}

fn typed_expr_to_rust(expr: &TypedExpr, ty: &str) -> String {
    match expr {
        TypedExpr::IntLit(n) => {
            let suffix = match ty { "float" | "length" => "f32", _ => "i32" };
            format!("Signal::new({}{suffix})", n)
        }
        TypedExpr::StringLit(s) => {
            format!("Signal::new(String::from(\"{}\"))", escape_str(s))
        }
        TypedExpr::Ident(s) => match ty {
            // `true`/`false` are valid Rust bool literals — no String wrapping.
            "bool"            => format!("Signal::new({})", s),
            _                 => format!("Signal::new(String::from({}))", s),
        },
        _ => default_signal_init(ty).to_string(),
    }
}

fn typed_expr_raw(expr: &TypedExpr) -> String {
    match expr {
        TypedExpr::IntLit(n)            => n.to_string(),
        TypedExpr::StringLit(s)         => format!("\"{}\"", escape_str(s)),
        TypedExpr::LengthLit(f)         => format!("{}f32", f),
        TypedExpr::Ident(s)             => s.clone(),
        TypedExpr::ColorLit { r, g, b } => format!("Color::rgb({}, {}, {})", r, g, b),
        _ => "()".to_string(),
    }
}

fn color_typed_to_rust(expr: &TypedExpr) -> String {
    match expr {
        TypedExpr::ColorLit { r, g, b } => format!("Color::rgb({}, {}, {})", r, g, b),
        TypedExpr::Ident(s)             => s.clone(),
        _ => "Color::WHITE".to_string(),
    }
}

fn build_format_string(parts: &[InterpolPart]) -> String {
    parts.iter().map(|p| match p {
        InterpolPart::Literal(s) => s.replace('{', "{{").replace('}', "}}"),
        InterpolPart::Var(_)     => "{}".to_string(),
    }).collect()
}

fn build_format_args(parts: &[InterpolPart]) -> String {
    let vars: Vec<&str> = parts.iter()
        .filter_map(|p| if let InterpolPart::Var(v) = p { Some(v.as_str()) } else { None })
        .collect();
    vars.iter().map(|_| "n").collect::<Vec<_>>().join(", ")
}

/// Locate a binding by `prop_name`, evaluate it, and emit a `Signal::new(...)` local
/// variable. Returns the variable name to pass to the widget constructor.
///
/// If the binding references `self.X` it is desugared to the local variable form.
/// If no binding is found, a zero-valued default signal is emitted.
fn find_signal_binding(
    bindings: &[Binding],
    prop_name: &str,
    widget_ty: &str,
    stmts: &mut Vec<String>,
    st: &mut CompState,
) -> String {
    let sig_var = format!("sig_{}_{}", prop_name, st.sub_counter);
    st.sub_counter += 1;

    if let Some(b) = bindings.iter().find(|b| b.property == prop_name) {
        match &b.value {
            Expr::Raw(r) => {
                // If the raw text is a bare `self.X` reference, emit a `.clone()` of
                // the local variable rather than wrapping it in Signal::new again.
                let desugared = desugar_prop_refs(&r.text);
                // A desugared prop ref looks like `*foo.get()` — detect by `*` prefix.
                if desugared.starts_with('*') && desugared.ends_with(".get()") {
                    // Strip leading `*` and trailing `.get()` to get the signal variable name.
                    let signal_name = &desugared[1..desugared.len() - 6];
                    stmts.push(format!(
                        "        let {} = {}.clone();",
                        sig_var, signal_name
                    ));
                } else {
                    stmts.push(format!(
                        "        let {} = Signal::new({});",
                        sig_var, desugared
                    ));
                }
            }
            Expr::SelfProp(prop) => {
                // Typed `self.prop` — clone the local Signal variable.
                stmts.push(format!("        let {} = {}.clone();", sig_var, prop));
            }
            other => {
                let rs_expr = compile_expr(other, ExprCtx::BuildFn);
                stmts.push(format!(
                    "        let {} = Signal::new({});",
                    sig_var, rs_expr
                ));
            }
        }
    } else {
        // No binding found — determine a sensible zero-value default for the widget.
        let default_val = match (widget_ty, prop_name) {
            ("CheckBox", "checked")  => "false".to_string(),
            ("TextEdit", "text")     => "String::new()".to_string(),
            ("ListView", "items")    => "alloc::vec::Vec::<String>::new()".to_string(),
            _                        => "0.0f32".to_string(),
        };
        stmts.push(format!(
            "        let {} = Signal::new({});",
            sig_var, default_val
        ));
    }

    sig_var
}

fn find_binding_f32(bindings: &[Binding], prop: &str) -> Option<f32> {
    bindings.iter().find(|b| b.property == prop).and_then(|b| {
        match &b.value {
            Expr::Raw(r) => match eval_binding(&r.text, prop, "") {
                TypedExpr::LengthLit(f) => Some(f),
                TypedExpr::IntLit(n)    => Some(n as f32),
                _ => None,
            },
            Expr::Literal(crate::ast::Literal::Int(n))   => Some(*n as f32),
            Expr::Literal(crate::ast::Literal::Float(f)) => Some(*f as f32),
            _ => None,
        }
    })
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Convert an `Expr` to a raw text string for use in legacy helpers that take `&str`.
///
/// For `Raw`, returns the stored text directly. For typed variants, uses `compile_expr`
/// in `BuildFn` context to produce equivalent Rust source.
fn expr_as_raw_text(expr: &Expr) -> String {
    match expr {
        Expr::Raw(r) => r.text.clone(),
        other => compile_expr(other, ExprCtx::BuildFn),
    }
}

/// Convert a typed `Expr` to a `TypedExpr` for use in `typed_expr_to_rust`.
///
/// Bridges new typed AST nodes into the existing `TypedExpr` → Signal::new(...) path
/// so property-type suffixes (`i32`, `f32`) are emitted consistently.
fn expr_to_typed(expr: &Expr, ty_hint: &str) -> TypedExpr {
    use crate::ast::Literal;
    match expr {
        Expr::Literal(Literal::Int(n)) => TypedExpr::IntLit(*n),
        Expr::Literal(Literal::Float(f)) => TypedExpr::LengthLit(*f as f32),
        Expr::Literal(Literal::Bool(b)) => TypedExpr::Ident(b.to_string()),
        Expr::Literal(Literal::Str(s)) => {
            // Re-use eval's string parsing so interpolated strings get the right type.
            eval_binding(&format!("{:?}", s), ty_hint, "")
        }
        Expr::Ident(s) => TypedExpr::Ident(s.clone()),
        other => TypedExpr::Ident(compile_expr(other, ExprCtx::BuildFn)),
    }
}

/// G1 heuristic: replace `self . ident` with `*self.ident.get()` in raw expressions.
///
/// `parse_until_lbrace` joins tokens with single spaces, so the `.vi` source
/// `self.show_panel` becomes the three-token sequence `self`, `.`, `show_panel`
/// which is joined into the string `"self . show_panel"`.  We must match that
/// spaced form.  If a caller produces the compact form `self.ident` directly
/// (e.g. in unit tests) that is also handled.
///
/// Does not handle `self.` inside string literals. Use P10 proper AST desugaring
/// when that matters.
fn desugar_prop_refs(s: &str) -> String {
    let mut result    = String::new();
    let mut remaining = s;

    while !remaining.is_empty() {
        // Prefer the spaced form `self . ident` (what the token-joiner emits).
        // Fall back to the compact form `self.ident` for direct usage.
        let (matched_prefix, idx) = if let Some(i) = remaining.find("self . ") {
            ("self . ", i)
        } else if let Some(i) = remaining.find("self.") {
            ("self.", i)
        } else {
            break;
        };

        result.push_str(&remaining[..idx]);
        let after = &remaining[idx + matched_prefix.len()..];

        // Skip any stray leading spaces before the identifier.
        let after_trimmed = after.trim_start_matches(' ');
        let trim_offset   = after.len() - after_trimmed.len();

        let ident_end = after_trimmed
            .char_indices()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);

        if ident_end > 0 {
            let ident = &after_trimmed[..ident_end];
            // build() is a static fn — properties are local variables, not self fields.
            result.push_str("*");
            result.push_str(ident);
            result.push_str(".get()");
            remaining = &after_trimmed[ident_end..];
        } else {
            // No identifier — emit prefix literally to avoid an infinite loop.
            result.push_str(matched_prefix);
            remaining = &after[trim_offset..];
        }
    }
    result.push_str(remaining);
    result
}
