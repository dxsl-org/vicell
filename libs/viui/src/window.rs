//! WindowChrome, WindowEvent, and WindowManager for desktop multi-window mode.
//!
//! App-side decoration: titlebar + close/min/max are rendered by ViUI into the
//! app's own ViSurface. The compositor sees opaque surfaces with no chrome knowledge.

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use ostd::display::{wait_for_compositor, ViSurface};
use ostd::input::InputEvent;
use ostd::input::{KeyState, KeySym};
use ostd::syscall::sys_recv;

use api::display::PixelFormat;

use crate::canvas::{Color, FramebufferCanvas, TextStyle, ViCanvas};
use crate::event::{Event, KeyCode, Modifiers, MouseButton};
use crate::layout::{Point, Rect};
use crate::theme::DARK_THEME;
use crate::widget::{PaintCx, ViWidget, WidgetTree};

// ─── Chrome geometry constants ───────────────────────────────────────────────

const TITLEBAR_H:  f32 = 28.0;
const BTN_SIZE:    f32 = 16.0;
const BTN_MARGIN:  f32 = 6.0;
const CHROME_BG:   Color = Color::rgb(35, 35, 50);
const CHROME_BTN_CLOSE: Color = Color::rgb(200, 60, 60);
const CHROME_BTN_MIN:   Color = Color::rgb(200, 160, 40);
const CHROME_BTN_MAX:   Color = Color::rgb(60, 160, 80);
const CHROME_BORDER:    Color = Color::rgb(60, 60, 90);
const TITLE_CLR:        Color = Color::WHITE;

// ─── WindowEvent ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum WindowEvent {
    Close,
    Minimize,
    Maximize,
    DragMove { dx: f32, dy: f32 },
}

// ─── WindowChrome ─────────────────────────────────────────────────────────────

/// Titlebar + close/min/max buttons rendered client-side into the surface.
pub struct WindowChrome {
    pub title:    String,
    drag_start:   Option<Point>,  // Some(initial press pos) during drag
}

impl WindowChrome {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), drag_start: None }
    }

    /// Titlebar drag area (screen-local to the surface, Y starts at 0).
    pub fn titlebar_rect(&self, surf_w: u32) -> Rect {
        Rect::new(0.0, 0.0, surf_w as f32 - (BTN_SIZE + BTN_MARGIN) * 3.0, TITLEBAR_H)
    }

    /// Close button rect (surface-local).
    pub fn close_btn_rect(&self, surf_w: u32) -> Rect {
        let x = surf_w as f32 - BTN_MARGIN - BTN_SIZE;
        Rect::new(x, (TITLEBAR_H - BTN_SIZE) / 2.0, BTN_SIZE, BTN_SIZE)
    }

    pub fn max_btn_rect(&self, surf_w: u32) -> Rect {
        let x = surf_w as f32 - BTN_MARGIN * 2.0 - BTN_SIZE * 2.0;
        Rect::new(x, (TITLEBAR_H - BTN_SIZE) / 2.0, BTN_SIZE, BTN_SIZE)
    }

    pub fn min_btn_rect(&self, surf_w: u32) -> Rect {
        let x = surf_w as f32 - BTN_MARGIN * 3.0 - BTN_SIZE * 3.0;
        Rect::new(x, (TITLEBAR_H - BTN_SIZE) / 2.0, BTN_SIZE, BTN_SIZE)
    }

    /// Paint the titlebar onto `canvas` (writes from y=0).
    pub fn paint(&self, canvas: &mut dyn ViCanvas) {
        let w = canvas.width();
        let bg = Rect::new(0.0, 0.0, w as f32, TITLEBAR_H);
        canvas.fill_rect(bg, CHROME_BG);

        // Bottom border
        canvas.draw_line(
            Point::new(0.0, TITLEBAR_H - 1.0),
            Point::new(w as f32, TITLEBAR_H - 1.0),
            CHROME_BORDER,
        );

        // Title text (left-padded)
        canvas.draw_text(Point::new(8.0, (TITLEBAR_H - 8.0) / 2.0), &self.title,
            TextStyle { color: TITLE_CLR, size_px: 0 });

        // Buttons: close, max, min (right-aligned)
        canvas.fill_rect(self.close_btn_rect(w), CHROME_BTN_CLOSE);
        canvas.fill_rect(self.max_btn_rect(w),   CHROME_BTN_MAX);
        canvas.fill_rect(self.min_btn_rect(w),   CHROME_BTN_MIN);
    }

    /// Process a pointer event at `surface-local` pos. Returns a WindowEvent if chrome handled it.
    pub fn event(&mut self, e: &Event, surf_w: u32) -> Option<WindowEvent> {
        match e {
            Event::MousePress { pos, button: MouseButton::Left } => {
                if self.close_btn_rect(surf_w).contains(*pos) { return Some(WindowEvent::Close); }
                if self.max_btn_rect(surf_w).contains(*pos)   { return Some(WindowEvent::Maximize); }
                if self.min_btn_rect(surf_w).contains(*pos)   { return Some(WindowEvent::Minimize); }
                if self.titlebar_rect(surf_w).contains(*pos) {
                    self.drag_start = Some(*pos);
                }
                None
            }
            Event::MouseMove { pos } => {
                if let Some(start) = self.drag_start {
                    let dx = pos.x - start.x;
                    let dy = pos.y - start.y;
                    return Some(WindowEvent::DragMove { dx, dy });
                }
                None
            }
            Event::MouseRelease { button: MouseButton::Left, .. } => {
                self.drag_start = None;
                None
            }
            _ => None,
        }
    }
}

// ─── Input translation ───────────────────────────────────────────────────────

pub(crate) fn keysym_from_u32(v: u32) -> Option<KeySym> {
    Some(match v {
        0x0000 => KeySym::Unknown,
        0x0001 => KeySym::Escape,
        0x0002 => KeySym::Return,
        0x0003 => KeySym::Backspace,
        0x0004 => KeySym::Tab,
        0x0005 => KeySym::Delete,
        0x0006 => KeySym::Insert,
        0x0010 => KeySym::Up,
        0x0011 => KeySym::Down,
        0x0012 => KeySym::Left,
        0x0013 => KeySym::Right,
        0x0020 => KeySym::Home,
        0x0021 => KeySym::End,
        0x0022 => KeySym::PageUp,
        0x0023 => KeySym::PageDown,
        0x0101 => KeySym::F1,  0x0102 => KeySym::F2,  0x0103 => KeySym::F3,
        0x0104 => KeySym::F4,  0x0105 => KeySym::F5,  0x0106 => KeySym::F6,
        0x0107 => KeySym::F7,  0x0108 => KeySym::F8,  0x0109 => KeySym::F9,
        0x010A => KeySym::F10, 0x010B => KeySym::F11, 0x010C => KeySym::F12,
        _ => return None,
    })
}

/// Decode a raw 64-byte IPC buffer into an `InputEvent`.
pub(crate) fn decode_input_event(buf: &[u8; 64]) -> Option<InputEvent> {
    use ostd::input::{KeyEvent, Modifiers as ApiMod};
    match buf[0] {
        0 => {
            let ts   = u64::from_le_bytes([buf[1],buf[2],buf[3],buf[4],buf[5],buf[6],buf[7],buf[8]]);
            let scan = u32::from_le_bytes([buf[9],buf[10],buf[11],buf[12]]);
            let ksym_raw = u32::from_le_bytes([buf[13],buf[14],buf[15],buf[16]]);
            let character = u32::from_le_bytes([buf[17],buf[18],buf[19],buf[20]]);
            let mods  = ApiMod(buf[21]);
            let state = match buf[22] { 1 => KeyState::Pressed, 2 => KeyState::Repeated, _ => KeyState::Released };
            let ksym  = keysym_from_u32(ksym_raw).unwrap_or(KeySym::Unknown);
            Some(InputEvent::Key(KeyEvent { timestamp_ticks: ts, scancode: scan, keysym: ksym, character, modifiers: mods, state, _pad: [0; 2] }))
        }
        1 => {
            let x  = i32::from_le_bytes([buf[1],buf[2],buf[3],buf[4]]);
            let y  = i32::from_le_bytes([buf[5],buf[6],buf[7],buf[8]]);
            let dx = i32::from_le_bytes([buf[9],buf[10],buf[11],buf[12]]);
            let dy = i32::from_le_bytes([buf[13],buf[14],buf[15],buf[16]]);
            Some(InputEvent::MouseMove { x, y, dx, dy })
        }
        2 => {
            let btn = match buf[1] & 0x7 {
                0 => ostd::input::MouseButton::Left,
                1 => ostd::input::MouseButton::Right,
                2 => ostd::input::MouseButton::Middle,
                3 => ostd::input::MouseButton::Back,
                4 => ostd::input::MouseButton::Forward,
                _ => return None,
            };
            let state = match buf[2] { 1 => KeyState::Pressed, _ => KeyState::Released };
            Some(InputEvent::MouseButton { button: btn, state })
        }
        3 => {
            let dx = i32::from_le_bytes([buf[1],buf[2],buf[3],buf[4]]);
            let dy = i32::from_le_bytes([buf[5],buf[6],buf[7],buf[8]]);
            Some(InputEvent::MouseScroll { dx, dy })
        }
        _ => None,
    }
}

/// Translate an `InputEvent` to a `viui::Event`. `mouse_pos` is the current pointer position.
pub(crate) fn translate_input(ev: InputEvent, mouse_pos: Point) -> Option<Event> {
    match ev {
        InputEvent::MouseMove { x, y, .. } => Some(Event::MouseMove { pos: Point::new(x as f32, y as f32) }),
        InputEvent::MouseButton { button, state } => {
            let btn = match button {
                ostd::input::MouseButton::Left   => MouseButton::Left,
                ostd::input::MouseButton::Right  => MouseButton::Right,
                ostd::input::MouseButton::Middle => MouseButton::Middle,
                _ => return None,
            };
            if state == KeyState::Released {
                Some(Event::MouseRelease { pos: mouse_pos, button: btn })
            } else {
                Some(Event::MousePress { pos: mouse_pos, button: btn })
            }
        }
        InputEvent::MouseScroll { dy, .. } => Some(Event::Scroll { pos: mouse_pos, delta_y: dy as f32 }),
        InputEvent::Key(k) => {
            let mods = Modifiers {
                shift: k.modifiers.contains(ostd::input::Modifiers::SHIFT),
                ctrl:  k.modifiers.contains(ostd::input::Modifiers::CTRL),
                alt:   k.modifiers.contains(ostd::input::Modifiers::ALT),
            };
            if k.state == KeyState::Released { return None; }
            if let Some(ch) = k.char() {
                if !ch.is_control() { return Some(Event::Char(ch)); }
            }
            let key = match k.keysym {
                KeySym::Backspace => KeyCode::Backspace,
                KeySym::Delete    => KeyCode::Delete,
                KeySym::Return    => KeyCode::Enter,
                KeySym::Tab       => KeyCode::Tab,
                KeySym::Escape    => KeyCode::Escape,
                KeySym::Left      => KeyCode::Left,
                KeySym::Right     => KeyCode::Right,
                KeySym::Up        => KeyCode::Up,
                KeySym::Down      => KeyCode::Down,
                KeySym::Home      => KeyCode::Home,
                KeySym::End       => KeyCode::End,
                KeySym::PageUp    => KeyCode::PageUp,
                KeySym::PageDown  => KeyCode::PageDown,
                KeySym::F1  => KeyCode::F(1),  KeySym::F2  => KeyCode::F(2),
                KeySym::F3  => KeyCode::F(3),  KeySym::F4  => KeyCode::F(4),
                KeySym::F5  => KeyCode::F(5),  KeySym::F6  => KeyCode::F(6),
                KeySym::F7  => KeyCode::F(7),  KeySym::F8  => KeyCode::F(8),
                KeySym::F9  => KeyCode::F(9),  KeySym::F10 => KeyCode::F(10),
                KeySym::F11 => KeyCode::F(11), KeySym::F12 => KeyCode::F(12),
                _ => return None,
            };
            Some(Event::KeyPress { key, modifiers: mods })
        }
    }
}

// ─── ManagedWindow ───────────────────────────────────────────────────────────

pub struct WindowId(usize);

struct ManagedWindow {
    surf:   ViSurface,
    chrome: WindowChrome,
    tree:   WidgetTree,
    x:      i32,
    y:      i32,
}

impl ManagedWindow {
    fn repaint(&mut self) {
        let w = self.surf.width();
        let h = self.surf.height();
        let stride = self.surf.stride() as u32;
        let pixels = self.surf.pixels_mut();
        let mut canvas = FramebufferCanvas::new(pixels, stride, w, h);

        // Chrome occupies top TITLEBAR_H rows; content paints below
        self.chrome.paint(&mut canvas);

        let content_origin = Point::new(0.0, TITLEBAR_H);
        let mut cx = PaintCx::with_theme(&mut canvas, &DARK_THEME);
        cx.origin = content_origin;
        self.tree.paint(&mut cx);

        self.surf.damage_all();
    }
}

// ─── WindowManager ───────────────────────────────────────────────────────────

pub struct WindowManager {
    windows:   Vec<ManagedWindow>,
    focused:   usize,
    comp_tid:  usize,
    mouse_pos: Point,
}

impl WindowManager {
    /// Connect to the compositor and create an empty manager.
    pub fn connect() -> Self {
        Self {
            windows:   Vec::new(),
            focused:   0,
            comp_tid:  wait_for_compositor(),
            mouse_pos: Point::ZERO,
        }
    }

    /// Open a new window. Returns its handle.
    pub fn open(&mut self, title: &str, w: u32, h: u32, content: Box<dyn ViWidget>) -> WindowId {
        let surf = ViSurface::create(self.comp_tid, w, h + TITLEBAR_H as u32, PixelFormat::Bgra8888)
            .expect("ViSurface::create failed");
        let chrome = WindowChrome::new(title);
        let mut tree = WidgetTree::rebuild(content);
        tree.layout(crate::layout::Size { w: w as f32, h: h as f32 });
        let id = WindowId(self.windows.len());
        self.windows.push(ManagedWindow { surf, chrome, tree, x: 0, y: 0 });
        // Repaint immediately
        let win = self.windows.last_mut().unwrap();
        win.repaint();
        id
    }

    /// Close a window by handle.
    pub fn close(&mut self, id: WindowId) {
        if id.0 < self.windows.len() {
            self.windows.remove(id.0);
            if self.focused >= self.windows.len() && !self.windows.is_empty() {
                self.focused = self.windows.len() - 1;
            }
        }
    }

    /// Run the event loop (blocks forever).
    pub fn event_loop(&mut self) -> ! {
        loop {
            let mut buf = [0u8; 64];
            if let ostd::syscall::SyscallResult::Ok(_) = sys_recv(0, &mut buf) {
                if let Some(raw_ev) = decode_input_event(&buf) {
                    // Track mouse position
                    if let InputEvent::MouseMove { x, y, .. } = raw_ev {
                        self.mouse_pos = Point::new(x as f32, y as f32);
                    }
                    if let Some(ev) = translate_input(raw_ev, self.mouse_pos) {
                        self.dispatch(ev);
                    }
                }
            }
        }
    }

    fn dispatch(&mut self, ev: Event) {
        let Some(win) = self.windows.get_mut(self.focused) else { return };
        let w = win.surf.width();

        // Let chrome handle it first (drag, buttons)
        if let Some(chrome_ev) = win.chrome.event(&ev, w) {
            match chrome_ev {
                WindowEvent::Close => {
                    let id = WindowId(self.focused);
                    self.close(id);
                    return;
                }
                WindowEvent::DragMove { dx, dy } => {
                    win.x += dx as i32;
                    win.y += dy as i32;
                    win.surf.move_to(win.x, win.y);
                    return;
                }
                WindowEvent::Minimize | WindowEvent::Maximize => { /* TODO P08+ */ return; }
            }
        }

        // Translate event to content-local coords (subtract chrome height)
        let content_ev = match &ev {
            Event::MouseMove { pos }           => Event::MouseMove { pos: Point::new(pos.x, pos.y - TITLEBAR_H) },
            Event::MousePress { pos, button }  => Event::MousePress { pos: Point::new(pos.x, pos.y - TITLEBAR_H), button: *button },
            Event::MouseRelease { pos, button } => Event::MouseRelease { pos: Point::new(pos.x, pos.y - TITLEBAR_H), button: *button },
            Event::Scroll { pos, delta_y }     => Event::Scroll { pos: Point::new(pos.x, pos.y - TITLEBAR_H), delta_y: *delta_y },
            other => other.clone(),
        };

        let dirty = win.tree.dispatch_event(&content_ev);
        if dirty { win.repaint(); }
    }
}
