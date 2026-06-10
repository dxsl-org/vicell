// Minimal EVDEV Scancode to ASCII mapping (US QWERTY)

pub const EV_KEY: u16 = 1;
pub const EV_REL: u16 = 2;
pub const EV_ABS: u16 = 3;

pub struct InputState {
    pub shift_pressed: bool,
    pub ctrl_pressed: bool,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            shift_pressed: false,
            ctrl_pressed: false,
        }
    }
}

pub static mut STATE: InputState = InputState {
    shift_pressed: false,
    ctrl_pressed: false,
};

pub fn scancode_to_ascii(code: u16, value: u32) -> Option<char> {
    // value: 1 = pressed, 0 = released, 2 = repeat
    let pressed = value > 0;
    log::debug!(
        "Mapping scancode: code={}, value={}, pressed={}",
        code,
        value,
        pressed
    );

    match code {
        42 | 54 => {
            // Left Shift, Right Shift
            unsafe {
                STATE.shift_pressed = pressed;
            }
            return None;
        }
        29 | 97 => {
            // Left Ctrl, Right Ctrl
            unsafe {
                STATE.ctrl_pressed = pressed;
            }
            return None;
        }
        _ => {}
    }

    if !pressed {
        return None;
    }

    let shift = unsafe { STATE.shift_pressed };

    match code {
        // Numbers
        2 => {
            if shift {
                Some('!')
            } else {
                Some('1')
            }
        }
        3 => {
            if shift {
                Some('@')
            } else {
                Some('2')
            }
        }
        4 => {
            if shift {
                Some('#')
            } else {
                Some('3')
            }
        }
        5 => {
            if shift {
                Some('$')
            } else {
                Some('4')
            }
        }
        6 => {
            if shift {
                Some('%')
            } else {
                Some('5')
            }
        }
        7 => {
            if shift {
                Some('^')
            } else {
                Some('6')
            }
        }
        8 => {
            if shift {
                Some('&')
            } else {
                Some('7')
            }
        }
        9 => {
            if shift {
                Some('*')
            } else {
                Some('8')
            }
        }
        10 => {
            if shift {
                Some('(')
            } else {
                Some('9')
            }
        }
        11 => {
            if shift {
                Some(')')
            } else {
                Some('0')
            }
        }
        12 => {
            if shift {
                Some('_')
            } else {
                Some('-')
            }
        }
        13 => {
            if shift {
                Some('+')
            } else {
                Some('=')
            }
        }

        // Rows
        16 => {
            if shift {
                Some('Q')
            } else {
                Some('q')
            }
        }
        17 => {
            if shift {
                Some('W')
            } else {
                Some('w')
            }
        }
        18 => {
            if shift {
                Some('E')
            } else {
                Some('e')
            }
        }
        19 => {
            if shift {
                Some('R')
            } else {
                Some('r')
            }
        }
        20 => {
            if shift {
                Some('T')
            } else {
                Some('t')
            }
        }
        21 => {
            if shift {
                Some('Y')
            } else {
                Some('y')
            }
        }
        22 => {
            if shift {
                Some('U')
            } else {
                Some('u')
            }
        }
        23 => {
            if shift {
                Some('I')
            } else {
                Some('i')
            }
        }
        24 => {
            if shift {
                Some('O')
            } else {
                Some('o')
            }
        }
        25 => {
            if shift {
                Some('P')
            } else {
                Some('p')
            }
        }
        30 => {
            if shift {
                Some('A')
            } else {
                Some('a')
            }
        }
        31 => {
            if shift {
                Some('S')
            } else {
                Some('s')
            }
        }
        32 => {
            if shift {
                Some('D')
            } else {
                Some('d')
            }
        }
        33 => {
            if shift {
                Some('F')
            } else {
                Some('f')
            }
        }
        34 => {
            if shift {
                Some('G')
            } else {
                Some('g')
            }
        }
        35 => {
            if shift {
                Some('H')
            } else {
                Some('h')
            }
        }
        36 => {
            if shift {
                Some('J')
            } else {
                Some('j')
            }
        }
        37 => {
            if shift {
                Some('K')
            } else {
                Some('k')
            }
        }
        38 => {
            if shift {
                Some('L')
            } else {
                Some('l')
            }
        }
        44 => {
            if shift {
                Some('Z')
            } else {
                Some('z')
            }
        }
        45 => {
            if shift {
                Some('X')
            } else {
                Some('x')
            }
        }
        46 => {
            if shift {
                Some('C')
            } else {
                Some('c')
            }
        }
        47 => {
            if shift {
                Some('V')
            } else {
                Some('v')
            }
        }
        48 => {
            if shift {
                Some('B')
            } else {
                Some('b')
            }
        }
        49 => {
            if shift {
                Some('N')
            } else {
                Some('n')
            }
        }
        50 => {
            if shift {
                Some('M')
            } else {
                Some('m')
            }
        }

        // Special
        28 => Some('\n'),     // Enter
        57 => Some(' '),      // Space
        14 => Some('\u{08}'), // Backspace (typical)

        // Punctuation
        51 => {
            if shift {
                Some('<')
            } else {
                Some(',')
            }
        }
        52 => {
            if shift {
                Some('>')
            } else {
                Some('.')
            }
        }
        53 => {
            if shift {
                Some('?')
            } else {
                Some('/')
            }
        }
        39 => {
            if shift {
                Some(':')
            } else {
                Some(';')
            }
        }
        40 => {
            if shift {
                Some('"')
            } else {
                Some('\'')
            }
        }
        26 => {
            if shift {
                Some('{')
            } else {
                Some('[')
            }
        }
        27 => {
            if shift {
                Some('}')
            } else {
                Some(']')
            }
        }
        43 => {
            if shift {
                Some('|')
            } else {
                Some('\\')
            }
        }

        _ => None,
    }
}
