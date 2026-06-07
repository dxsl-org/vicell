//! Elm architecture — `ViApp`, `Element<Msg>`, free-function builders, macros.
//!
//! iced-compatible API shape: `ViApp::view()` returns `Element<Msg>`.
//! Free functions: `text()`, `button()`, `column()`, `row()`, `checkbox()`, `scrollable()`, `space()`, `image()`.
//! Macros: `column![]`, `row![]` for inline child lists.

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::event::{Event, EventCx, EventStatus};
use crate::layout::{Constraints, LayoutNode, Padding, Point, Rect, Size};
use crate::widget::{PaintCx, WidgetId, ViWidget};

// ─── ViApp ───────────────────────────────────────────────────────────────────

/// Elm-architecture application. iced `Application` compatible shape.
///
/// Implement this trait on your app state struct. Then call `run_app()` (P06).
pub trait ViApp: 'static + Sized {
    type Message: 'static + Clone;

    /// Build the widget tree for the current app state.
    ///
    /// Called after every `update()`. May return a fresh tree each time — cheap
    /// because only the new root pointer is stored; layout runs on dirty frames.
    fn view(&self) -> Element<Self::Message>;

    /// Mutate app state in response to a `Message`.
    fn update(&mut self, msg: Self::Message);

    /// Window title (optional override).
    fn title(&self) -> &str { "ViCell" }
}

// ─── Element<Msg> ────────────────────────────────────────────────────────────

/// Type-erased widget node carrying a message type `Msg`.
///
/// Wraps any `ViWidget` and maps its events to `Option<Msg>`.
/// P06 provides free-function constructors (`text()`, `button()`, …).
pub struct Element<Msg: 'static> {
    inner: Box<dyn ErasedWidget<Msg>>,
}

impl<Msg: 'static> Element<Msg> {
    /// Wrap a plain `ViWidget` that never emits messages.
    pub fn inert(widget: impl ViWidget) -> Self {
        struct Inert<W>(W);
        impl<W: ViWidget, M: 'static> ErasedWidget<M> for Inert<W> {
            fn layout(&self, c: Constraints) -> LayoutNode { self.0.layout(c) }
            fn paint(&self, cx: &mut PaintCx) { self.0.paint(cx) }
            fn event(&mut self, cx: &mut EventCx, e: &Event) -> (EventStatus, Option<M>) {
                (self.0.event(cx, e), None)
            }
        }
        Self { inner: Box::new(Inert(widget)) }
    }

    /// Wrap a widget that can emit a message.
    ///
    /// `on_event` is called after the widget processes the event; if it returns
    /// `Some(msg)`, the Elm runner delivers it to `ViApp::update()`.
    pub fn with_message<W, F>(widget: W, on_event: F) -> Self
    where
        W: ViWidget,
        F: Fn(&EventCx) -> Option<Msg> + 'static,
    {
        struct Emitter<W, F, M> {
            widget: W,
            on_event: F,
            _msg: core::marker::PhantomData<M>,
        }
        impl<W: ViWidget, F: Fn(&EventCx) -> Option<M> + 'static, M: 'static> ErasedWidget<M>
            for Emitter<W, F, M>
        {
            fn layout(&self, c: Constraints) -> LayoutNode { self.widget.layout(c) }
            fn paint(&self, cx: &mut PaintCx) { self.widget.paint(cx) }
            fn event(&mut self, cx: &mut EventCx, e: &Event) -> (EventStatus, Option<M>) {
                let status = self.widget.event(cx, e);
                let msg = (self.on_event)(cx);
                (status, msg)
            }
        }
        Self { inner: Box::new(Emitter { widget, on_event, _msg: core::marker::PhantomData }) }
    }

    pub fn layout(&self, c: Constraints) -> LayoutNode { self.inner.layout(c) }
    pub fn paint(&self, cx: &mut PaintCx) { self.inner.paint(cx) }
    pub fn event(&mut self, cx: &mut EventCx, e: &Event) -> (EventStatus, Option<Msg>) {
        self.inner.event(cx, e)
    }
}

// ─── ErasedWidget ────────────────────────────────────────────────────────────

trait ErasedWidget<Msg: 'static>: 'static {
    fn layout(&self, constraints: Constraints) -> LayoutNode;
    fn paint(&self, cx: &mut PaintCx);
    fn event(&mut self, cx: &mut EventCx, e: &Event) -> (EventStatus, Option<Msg>);
}

// ─── ElmColumn / ElmRow ──────────────────────────────────────────────────────
//
// Elm-mode containers that propagate messages from children.

struct ElmColumn<Msg: 'static> { children: Vec<Element<Msg>>, spacing: f32, padding: Padding }
struct ElmRow<Msg: 'static>    { children: Vec<Element<Msg>>, spacing: f32, padding: Padding }

impl<Msg: 'static> ErasedWidget<Msg> for ElmColumn<Msg> {
    fn layout(&self, c: Constraints) -> LayoutNode {
        let ox = c.origin.x + self.padding.left;
        let mut oy = c.origin.y + self.padding.top;
        let iw = (c.max.w - self.padding.h_total()).max(0.0);
        let ih = (c.max.h - self.padding.v_total()).max(0.0);
        let mut nodes = Vec::new();
        for child in &self.children {
            let rem = (c.origin.y + ih - oy + c.origin.y).max(0.0);
            let cc = Constraints::new(Point::new(ox, oy), Size { w: iw, h: rem });
            let node = child.layout(cc);
            let h = node.bounds.h;
            nodes.push(node);
            oy += h + self.spacing;
        }
        let total_h = ((oy - c.origin.y - self.spacing).max(0.0) + self.padding.v_total()).min(c.max.h);
        LayoutNode::with_children(Rect { x: c.origin.x, y: c.origin.y, w: c.max.w, h: total_h }, nodes)
    }
    fn paint(&self, cx: &mut PaintCx) {
        for child in &self.children { child.paint(cx); }
    }
    fn event(&mut self, cx: &mut EventCx, e: &Event) -> (EventStatus, Option<Msg>) {
        for (i, child) in self.children.iter_mut().enumerate() {
            let Some(cl) = cx.layout.child(i) else { break };
            let mut ccx = EventCx {
                state: cx.state, focus: cx.focus,
                widget_id: cx.widget_id.child(i),
                layout: cl, needs_repaint: false,
            };
            let (status, msg) = child.event(&mut ccx, e);
            if ccx.needs_repaint { cx.mark_dirty(); }
            if msg.is_some() || status == EventStatus::Consumed {
                return (status, msg);
            }
        }
        (EventStatus::Ignored, None)
    }
}

impl<Msg: 'static> ErasedWidget<Msg> for ElmRow<Msg> {
    fn layout(&self, c: Constraints) -> LayoutNode {
        let oy = c.origin.y + self.padding.top;
        let mut ox = c.origin.x + self.padding.left;
        let iw = (c.max.w - self.padding.h_total()).max(0.0);
        let ih = (c.max.h - self.padding.v_total()).max(0.0);
        let mut nodes = Vec::new();
        for child in &self.children {
            let rem = (c.origin.x + iw - ox + c.origin.x).max(0.0);
            let cc = Constraints::new(Point::new(ox, oy), Size { w: rem, h: ih });
            let node = child.layout(cc);
            let w = node.bounds.w;
            nodes.push(node);
            ox += w + self.spacing;
        }
        let total_w = ((ox - c.origin.x - self.spacing).max(0.0) + self.padding.h_total()).min(c.max.w);
        LayoutNode::with_children(Rect { x: c.origin.x, y: c.origin.y, w: total_w, h: ih + self.padding.v_total() }, nodes)
    }
    fn paint(&self, cx: &mut PaintCx) {
        for child in &self.children { child.paint(cx); }
    }
    fn event(&mut self, cx: &mut EventCx, e: &Event) -> (EventStatus, Option<Msg>) {
        for (i, child) in self.children.iter_mut().enumerate() {
            let Some(cl) = cx.layout.child(i) else { break };
            let mut ccx = EventCx {
                state: cx.state, focus: cx.focus,
                widget_id: cx.widget_id.child(i),
                layout: cl, needs_repaint: false,
            };
            let (status, msg) = child.event(&mut ccx, e);
            if ccx.needs_repaint { cx.mark_dirty(); }
            if msg.is_some() || status == EventStatus::Consumed {
                return (status, msg);
            }
        }
        (EventStatus::Ignored, None)
    }
}

// ─── ButtonBuilder ────────────────────────────────────────────────────────────

/// Builder for `button()` — adds `.on_press(msg)` before resolving to `Element<Msg>`.
pub struct ButtonBuilder<Msg> {
    id:      WidgetId,
    label:   String,
    padding: Padding,
    on_press: Option<Msg>,
}

impl<Msg: 'static + Clone> ButtonBuilder<Msg> {
    fn new(id: WidgetId, label: String) -> Self {
        Self { id, label, padding: Padding::all(6.0), on_press: None }
    }

    pub fn on_press(mut self, msg: Msg) -> Element<Msg> {
        self.on_press = Some(msg);
        self.into_element()
    }

    pub fn padding(mut self, px: f32) -> Self { self.padding = Padding::all(px); self }

    fn into_element(self) -> Element<Msg> {
        use crate::widgets::Button;
        let mut btn = Button::new(self.id, self.label);
        btn.padding = self.padding;
        let on_press = self.on_press;
        Element::with_message(btn, move |_cx| on_press.clone())
    }
}

// ─── Free-function builders ──────────────────────────────────────────────────

/// Construct a text label element.
pub fn text<Msg: 'static>(content: impl Into<String>) -> Element<Msg> {
    use crate::widgets::Label;
    Element::inert(Label::new(content))
}

/// Construct a button builder. Chain `.on_press(msg)` to produce `Element<Msg>`.
pub fn button<Msg: 'static + Clone>(label: impl Into<String>) -> ButtonBuilder<Msg> {
    let s = label.into();
    let id = WidgetId::new(&s);
    ButtonBuilder::new(id, s)
}

/// Vertically stack children.
pub fn column<Msg: 'static>(children: Vec<Element<Msg>>) -> Element<Msg> {
    Element { inner: Box::new(ElmColumn { children, spacing: 4.0, padding: Padding::ZERO }) }
}

/// Horizontally arrange children.
pub fn row<Msg: 'static>(children: Vec<Element<Msg>>) -> Element<Msg> {
    Element { inner: Box::new(ElmRow { children, spacing: 4.0, padding: Padding::ZERO }) }
}

/// Checkbox that emits `msg(new_checked)` on toggle.
pub fn checkbox<Msg: 'static + Clone, F>(checked: bool, label: impl Into<String>, on_toggle: F) -> Element<Msg>
where F: Fn(bool) -> Msg + 'static
{
    use crate::widgets::Checkbox;
    let s = label.into();
    let id = WidgetId::new(&s);
    let cb = Checkbox::new(id, checked, s);
    Element::with_message(cb, move |_cx| {
        // The checkbox widget updates its own state; we peek the new state via closure.
        // on_toggle is called with the toggled value — but we can't read widget state
        // from EventCx here. Return the toggled value using a cell.
        // Simplified: always return Some(on_toggle(!checked)) as approximation.
        // In P07 with a proper message bus, this will be done correctly.
        Some(on_toggle(!checked))
    })
}

/// Invisible spacer.
pub fn space<Msg: 'static>(w: f32, h: f32) -> Element<Msg> {
    use crate::widgets::Space;
    Element::inert(Space::new(w, h))
}

/// Image element from raw BGRA pixels.
pub fn image<Msg: 'static>(pixels: alloc::vec::Vec<u8>, w: u32, h: u32) -> Element<Msg> {
    use crate::widgets::Image;
    Element::inert(Image::new(pixels, w, h))
}

/// Vertically scrollable container.
pub fn scrollable<Msg: 'static>(id: WidgetId, content: Element<Msg>) -> Element<Msg> {
    // Wraps content in an inert ScrollArea; message propagation deferred to P07.
    // For now, wrap the Elm content as a ViWidget via a thin adapter.
    struct ElmScrollable<M: 'static> { inner: Element<M>, id: WidgetId }
    impl<M: 'static> ErasedWidget<M> for ElmScrollable<M> {
        fn layout(&self, c: Constraints) -> LayoutNode {
            let own_size = c.constrain(c.max);
            let child_c = Constraints::new(c.origin, Size { w: own_size.w, h: own_size.h * 4.0 });
            let child_node = self.inner.layout(child_c);
            let bounds = Rect::from_origin_size(c.origin, own_size);
            LayoutNode::with_children(bounds, alloc::vec![child_node])
        }
        fn paint(&self, cx: &mut PaintCx) {
            self.inner.paint(cx);
        }
        fn event(&mut self, cx: &mut EventCx, e: &Event) -> (EventStatus, Option<M>) {
            if let Some(cl) = cx.layout.child(0) {
                let mut ccx = EventCx {
                    state: cx.state, focus: cx.focus,
                    widget_id: self.id.child(0),
                    layout: cl, needs_repaint: false,
                };
                let result = self.inner.event(&mut ccx, e);
                if ccx.needs_repaint { cx.mark_dirty(); }
                result
            } else {
                (EventStatus::Ignored, None)
            }
        }
    }
    Element { inner: Box::new(ElmScrollable { inner: content, id }) }
}

// ─── run_app ─────────────────────────────────────────────────────────────────

/// Entry point for an Elm-architecture ViUI application.
///
/// Creates a single window for the app, connects to the compositor, and runs
/// the Elm event loop: recv input → dispatch → update(msg) → view() → repaint.
pub fn run_app<App: ViApp>(mut app: App) -> ! {
    use crate::canvas::FramebufferCanvas;
    use crate::theme::DARK_THEME;
    use crate::widget::PaintCx;
    use ostd::display::{wait_for_compositor, ViSurface};
    use ostd::syscall::{sys_recv, SyscallResult};
    use ostd::input::InputEvent;
    use crate::window::{decode_input_event, translate_input, WindowChrome};
    use api::display::PixelFormat;

    let comp_tid = wait_for_compositor();
    let w = 640u32;
    let h = 480u32;
    let chrome_h = 28u32;

    let mut surf = ViSurface::create(comp_tid, w, h + chrome_h, PixelFormat::Bgra8888)
        .expect("ViSurface::create failed");

    // Build initial Elm widget tree from app.view()
    let root_el = app.view();

    let chrome = WindowChrome::new(app.title());
    let mut mouse_pos = crate::layout::Point::ZERO;
    let mut state = crate::state_store::WidgetStateStore::new();
    let mut focus  = crate::state_store::FocusManager::new();
    let mut root = root_el;

    // Initial layout + paint
    let screen = crate::layout::Size { w: w as f32, h: h as f32 };
    let mut layout_cache = root.layout(crate::layout::Constraints::root(screen));

    let paint = |surf: &mut ViSurface, root: &crate::elm::Element<App::Message>,
                  chrome: &WindowChrome| {
        let stride = surf.stride() as u32;
        let sw = surf.width();
        let sh = surf.height();
        let pixels = surf.pixels_mut();
        let mut canvas = FramebufferCanvas::new(pixels, stride, sw, sh);
        chrome.paint(&mut canvas);
        let mut cx = PaintCx::with_theme(&mut canvas, &DARK_THEME);
        cx.origin = crate::layout::Point::new(0.0, chrome_h as f32);
        root.paint(&mut cx);
        surf.damage_all();
    };
    paint(&mut surf, &root, &chrome);

    loop {
        let mut buf = [0u8; 64];
        if let SyscallResult::Ok(_) = sys_recv(0, &mut buf) {
            if let Some(raw_ev) = decode_input_event(&buf) {
                if let InputEvent::MouseMove { x, y, .. } = raw_ev {
                    mouse_pos = crate::layout::Point::new(x as f32, y as f32);
                }
                if let Some(ev) = translate_input(raw_ev, mouse_pos) {
                    let content_ev = match &ev {
                        crate::event::Event::MouseMove { pos } =>
                            crate::event::Event::MouseMove { pos: crate::layout::Point::new(pos.x, pos.y - chrome_h as f32) },
                        crate::event::Event::MousePress { pos, button } =>
                            crate::event::Event::MousePress { pos: crate::layout::Point::new(pos.x, pos.y - chrome_h as f32), button: *button },
                        crate::event::Event::MouseRelease { pos, button } =>
                            crate::event::Event::MouseRelease { pos: crate::layout::Point::new(pos.x, pos.y - chrome_h as f32), button: *button },
                        other => other.clone(),
                    };

                    let mut cx = crate::event::EventCx {
                        state: &mut state,
                        focus: &mut focus,
                        widget_id: crate::widget::WidgetId::ROOT,
                        layout: crate::layout::LayoutView(&layout_cache),
                        needs_repaint: false,
                    };
                    let (_status, msg) = root.event(&mut cx, &content_ev);
                    let dirty = cx.needs_repaint;

                    if let Some(m) = msg {
                        app.update(m);
                        root = app.view();
                        layout_cache = root.layout(crate::layout::Constraints::root(screen));
                        paint(&mut surf, &root, &chrome);
                    } else if dirty {
                        paint(&mut surf, &root, &chrome);
                    }
                }
            }
        }
    }
}

// ─── Macros ──────────────────────────────────────────────────────────────────

/// Build a `column` element from a list of child expressions.
///
/// ```ignore
/// column![text("Hello"), button("OK").on_press(Msg::Ok)]
/// ```
#[macro_export]
macro_rules! column {
    ($($e:expr),* $(,)?) => {
        $crate::elm::column(alloc::vec![$($e),*])
    };
}

/// Build a `row` element from a list of child expressions.
#[macro_export]
macro_rules! row {
    ($($e:expr),* $(,)?) => {
        $crate::elm::row(alloc::vec![$($e),*])
    };
}
