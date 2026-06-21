//! Tetris game state: board, pieces, rotation, scoring.

pub const COLS: usize = 10;
pub const ROWS: usize = 20;

// 7 piece types × 4 rotations as 4×4 bitmasks.
// Bit layout: bit 15=(col0,row0), bit 14=(col1,row0), ..., bit 0=(col3,row3).
// Values match standard Tetris Guideline spawn orientations.
const SHAPES: [[u16; 4]; 7] = [
    [0x0F00, 0x2222, 0x00F0, 0x4444], // I  bright cyan
    [0x8E00, 0x6440, 0x0E20, 0x44C0], // J  bright blue
    [0x2E00, 0x4460, 0x0E80, 0xC440], // L  yellow
    [0x6600, 0x6600, 0x6600, 0x6600], // O  brown/orange
    [0x6C00, 0x4620, 0x06C0, 0x8C40], // S  bright green
    [0x4E00, 0x4640, 0x0E40, 0x4C40], // T  bright magenta
    [0xC600, 0x2640, 0x0C60, 0x4C80], // Z  bright red
];

// BGRA colors per piece kind (format: 0xAARRGGBB = (A<<24)|(R<<16)|(G<<8)|B).
pub const PIECE_COLORS: [u32; 7] = [
    0xFF55FFFF, // I: bright cyan
    0xFF5555FF, // J: bright blue
    0xFFFFFF55, // L: yellow
    0xFFAA5500, // O: orange-brown
    0xFF55FF55, // S: bright green
    0xFFFF55FF, // T: bright magenta
    0xFFFF5555, // Z: bright red
];

pub const BG_COLOR:     u32 = 0xFF000000;
pub const BORDER_COLOR: u32 = 0xFF333333;
pub const TEXT_COLOR:   u32 = 0xFFFFFFFF;
pub const PANEL_COLOR:  u32 = 0xFF111111;
pub const GHOST_COLOR:  u32 = 0xFF2A2A2A;

/// Extract the 4 occupied (col, row) positions from a piece shape.
/// `col` / `row` are the board coordinates of the 4×4 grid's top-left corner.
pub fn piece_cells(kind: usize, rot: usize, col: i32, row: i32) -> [(i32, i32); 4] {
    let shape = SHAPES[kind][rot];
    let mut out = [(0i32, 0i32); 4];
    let mut n = 0;
    for r in 0..4i32 {
        for c in 0..4i32 {
            if shape & (1u16 << (15 - (r * 4 + c))) != 0 {
                out[n] = (col + c, row + r);
                n += 1;
            }
        }
    }
    out
}

pub fn can_fit(board: &[[u8; COLS]; ROWS], kind: usize, rot: usize, col: i32, row: i32) -> bool {
    for (x, y) in piece_cells(kind, rot, col, row) {
        if x < 0 || x >= COLS as i32 || y >= ROWS as i32 { return false; }
        if y >= 0 && board[y as usize][x as usize] != 0 { return false; }
    }
    true
}

pub struct Game {
    pub board:     [[u8; COLS]; ROWS], // 0=empty, 1-7=piece kind+1
    pub kind:      usize,
    pub rot:       usize,
    pub col:       i32,
    pub row:       i32,
    pub next_kind: usize,
    pub score:     u32,
    pub lines:     u32,
    pub level:     u32,
    pub over:      bool,
    drop_accum:    u32,
    // Deterministic pseudo-random state (xorshift32)
    rng:           u32,
}

impl Game {
    pub fn new(seed: u32) -> Self {
        let mut g = Self {
            board: [[0; COLS]; ROWS],
            kind: 0, rot: 0, col: 3, row: 0,
            next_kind: 0, score: 0, lines: 0, level: 1,
            over: false, drop_accum: 0,
            rng: if seed == 0 { 1 } else { seed },
        };
        g.next_kind = g.rand7();
        g.spawn();
        g
    }

    fn rand7(&mut self) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 7) as usize
    }

    fn spawn(&mut self) {
        self.kind = self.next_kind;
        self.next_kind = self.rand7();
        self.rot = 0;
        self.col = 3;
        self.row = -1;
        if !can_fit(&self.board, self.kind, self.rot, self.col, self.row) {
            self.over = true;
        }
    }

    pub fn move_left(&mut self) {
        if can_fit(&self.board, self.kind, self.rot, self.col - 1, self.row) {
            self.col -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if can_fit(&self.board, self.kind, self.rot, self.col + 1, self.row) {
            self.col += 1;
        }
    }

    pub fn rotate(&mut self) {
        let nr = (self.rot + 1) % 4;
        // Simple wall-kick: try col, col-1, col+1
        for dc in [0, -1, 1, -2, 2] {
            if can_fit(&self.board, self.kind, nr, self.col + dc, self.row) {
                self.rot = nr;
                self.col += dc;
                return;
            }
        }
    }

    pub fn soft_drop(&mut self) {
        if can_fit(&self.board, self.kind, self.rot, self.col, self.row + 1) {
            self.row += 1;
            self.drop_accum = 0;
        }
    }

    pub fn hard_drop(&mut self) {
        while can_fit(&self.board, self.kind, self.rot, self.col, self.row + 1) {
            self.row += 1;
        }
        self.lock();
    }

    fn lock(&mut self) {
        let color = (self.kind + 1) as u8;
        for (x, y) in piece_cells(self.kind, self.rot, self.col, self.row) {
            if y >= 0 { self.board[y as usize][x as usize] = color; }
        }
        self.clear_lines();
        self.spawn();
    }

    fn clear_lines(&mut self) {
        let mut cleared = 0u32;
        let mut r = ROWS;
        while r > 0 {
            r -= 1;
            if self.board[r].iter().all(|&c| c != 0) {
                self.board[..=r].rotate_right(1);
                self.board[0] = [0; COLS];
                cleared += 1;
            }
        }
        self.lines += cleared;
        self.score += [0, 100, 300, 500, 800][cleared.min(4) as usize] * self.level;
        self.level = self.lines / 10 + 1;
    }

    /// Advance game by `dt_ms` milliseconds (gravity tick).
    pub fn tick(&mut self, dt_ms: u32) {
        self.drop_accum += dt_ms;
        let interval = self.drop_interval();
        if self.drop_accum >= interval {
            self.drop_accum = 0;
            if can_fit(&self.board, self.kind, self.rot, self.col, self.row + 1) {
                self.row += 1;
            } else {
                self.lock();
            }
        }
    }

    fn drop_interval(&self) -> u32 {
        match self.level {
            1 => 800, 2 => 720, 3 => 630, 4 => 550, 5 => 470,
            6 => 380, 7 => 300, 8 => 220, 9 => 130, 10 => 100,
            11..=19 => 80, 20..=28 => 50, _ => 30,
        }
    }

    /// Ghost row: how far down the piece would fall.
    pub fn ghost_row(&self) -> i32 {
        let mut r = self.row;
        while can_fit(&self.board, self.kind, self.rot, self.col, r + 1) { r += 1; }
        r
    }
}
