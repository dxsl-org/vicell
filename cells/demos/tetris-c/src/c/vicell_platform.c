/*
 * vicell_platform.c — ViCell platform backend for Banaxi-Tech/Tetris-OS.
 *
 * Replaces the following files from tetris-os (DO NOT compile them):
 *   vga.c / vga.h       → vga_fill_rect, vga_clear, vga_present, vga_draw_*
 *   keyboard.c / .h     → keyboard_init, keyboard_get_key
 *   timer.c / .h        → timer_init, timer_get_ticks
 *   speaker.c / .h      → speaker_music_start, speaker_music_stop, speaker_music_tick
 *   main.c              → entry provided by Rust src/main.rs
 *
 * Rendering: VGA 320×200 scaled 3× → 960×600, centred in 1024×768 BGRA surface.
 * Font: 8×8 CP437 glyphs rendered by Rust (FONT8X8 from ostd::font).
 * Timing / input: thin stubs that call back into Rust.
 *
 * Geometry constants:
 *   VGA_W=320  VGA_H=200  SCALE=3
 *   SURF_W=1024 SURF_H=768
 *   XOFF=(1024-960)/2=32   YOFF=(768-600)/2=84
 */

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

/* ── Rust callbacks ─────────────────────────────────────────────────────────
 * These are #[no_mangle] extern "C" functions defined in src/main.rs.
 * They must exactly match the declarations below.
 */
extern uint32_t vicell_get_ticks_ms(void);
extern int      vicell_poll_key(void);
extern uint32_t *vicell_surface_ptr(void);
extern void     vicell_flush(void);
extern void     vicell_draw_char(uint32_t x, uint32_t y, uint8_t c, uint32_t bgra);

/* ── Key codes ──────────────────────────────────────────────────────────────
 * Must match the keyboard.h enum from Banaxi-Tech/Tetris-OS.
 * If the _Static_asserts below fail after cloning, update the KEY_* constants
 * in src/main.rs to match.
 */
#ifdef __has_include
#  if __has_include("tetris-os/keyboard.h")
#    include "tetris-os/keyboard.h"
     /* Validate that the Rust-side constants match (build fails with a clear
      * message if they don't — update KEY_* in src/main.rs accordingly). */
     _Static_assert(KEY_NONE   == 0, "Update KEY_NONE   in main.rs to match keyboard.h");
     _Static_assert(KEY_LEFT   == 1, "Update KEY_LEFT   in main.rs to match keyboard.h");
     _Static_assert(KEY_RIGHT  == 2, "Update KEY_RIGHT  in main.rs to match keyboard.h");
     _Static_assert(KEY_UP     == 3, "Update KEY_UP     in main.rs to match keyboard.h");
     _Static_assert(KEY_DOWN   == 4, "Update KEY_DOWN   in main.rs to match keyboard.h");
     _Static_assert(KEY_ENTER  == 5, "Update KEY_ENTER  in main.rs to match keyboard.h");
     _Static_assert(KEY_ESCAPE == 6, "Update KEY_ESCAPE in main.rs to match keyboard.h");
#  else
     /* Fallback definitions when repo is not yet cloned (cargo check). */
#    define KEY_NONE   0
#    define KEY_LEFT   1
#    define KEY_RIGHT  2
#    define KEY_UP     3
#    define KEY_DOWN   4
#    define KEY_ENTER  5
#    define KEY_ESCAPE 6
#  endif
#else
#  define KEY_NONE   0
#  define KEY_LEFT   1
#  define KEY_RIGHT  2
#  define KEY_UP     3
#  define KEY_DOWN   4
#  define KEY_ENTER  5
#  define KEY_ESCAPE 6
#endif

/* ── VGA palette ────────────────────────────────────────────────────────────
 * 16 CGA/VGA colours as BGRA8888 u32 (little-endian layout):
 *   byte[0]=B  byte[1]=G  byte[2]=R  byte[3]=A
 * u32 value = (A<<24)|(R<<16)|(G<<8)|B = 0xAARRGGBB
 */
static const uint32_t PALETTE[16] = {
    0xFF000000, /* 0  Black          R=0   G=0   B=0   */
    0xFF0000AA, /* 1  Blue           R=0   G=0   B=170 */
    0xFF00AA00, /* 2  Green          R=0   G=170 B=0   */
    0xFF00AAAA, /* 3  Cyan           R=0   G=170 B=170 */
    0xFFAA0000, /* 4  Red            R=170 G=0   B=0   */
    0xFFAA00AA, /* 5  Magenta        R=170 G=0   B=170 */
    0xFFAA5500, /* 6  Brown          R=170 G=85  B=0   */
    0xFFAAAAAA, /* 7  Light Gray     R=170 G=170 B=170 */
    0xFF555555, /* 8  Dark Gray      R=85  G=85  B=85  */
    0xFF5555FF, /* 9  Bright Blue    R=85  G=85  B=255 */
    0xFF55FF55, /* 10 Bright Green   R=85  G=255 B=85  */
    0xFF55FFFF, /* 11 Bright Cyan    R=85  G=255 B=255 */
    0xFFFF5555, /* 12 Bright Red     R=255 G=85  B=85  */
    0xFFFF55FF, /* 13 Bright Magenta R=255 G=85  B=255 */
    0xFFFFFF55, /* 14 Bright Yellow  R=255 G=255 B=85  */
    0xFFFFFFFF, /* 15 White          R=255 G=255 B=255 */
};

/* ── Geometry ────────────────────────────────────────────────────────────── */
#define VGA_W  320u
#define VGA_H  200u
#define SCALE  3u
#define SURF_W 1024u
#define SURF_H 768u
#define XOFF   ((SURF_W - VGA_W * SCALE) / 2u)   /* 32 */
#define YOFF   ((SURF_H - VGA_H * SCALE) / 2u)   /* 84 */

/* ── VGA functions ──────────────────────────────────────────────────────── */

void vga_init(void) { /* no-op: surface created in Rust before tetris_cell_run() */ }

void vga_clear(uint8_t color) {
    uint32_t *buf = vicell_surface_ptr();
    if (!buf) return;
    uint32_t bgra = PALETTE[color & 0xFu];
    /* Fill entire surface (letterbox + game area) */
    for (uint32_t i = 0; i < SURF_W * SURF_H; i++) {
        buf[i] = bgra;
    }
}

void vga_fill_rect(uint32_t x, uint32_t y, uint32_t w, uint32_t h, uint8_t color) {
    uint32_t *buf = vicell_surface_ptr();
    if (!buf) return;
    uint32_t bgra = PALETTE[color & 0xFu];
    for (uint32_t dy = 0; dy < h * SCALE; dy++) {
        uint32_t sy = YOFF + y * SCALE + dy;
        if (sy >= SURF_H) break;
        uint32_t *row = buf + sy * SURF_W;
        for (uint32_t dx = 0; dx < w * SCALE; dx++) {
            uint32_t sx = XOFF + x * SCALE + dx;
            if (sx >= SURF_W) continue;
            row[sx] = bgra;
        }
    }
}

void vga_present(void) {
    vicell_flush();
}

/* Character rendering delegates to the Rust font engine (ostd::font::FONT8X8).
 * x, y are in VGA pixel coordinates; vicell_draw_char receives scaled screen coords. */
void vga_draw_char(uint32_t x, uint32_t y, char c, uint8_t color) {
    vicell_draw_char(
        XOFF + x * SCALE,
        YOFF + y * SCALE,
        (uint8_t)c,
        PALETTE[color & 0xFu]
    );
}

void vga_draw_string(uint32_t x, uint32_t y, const char *str, uint8_t color) {
    if (!str) return;
    while (*str) {
        vga_draw_char(x, y, *str++, color);
        x += 8u; /* 8 VGA pixels per glyph column */
    }
}

void vga_draw_number(uint32_t x, uint32_t y, uint32_t num, uint8_t color) {
    char buf[12];
    int i = 11;
    buf[i] = '\0';
    if (num == 0u) {
        buf[--i] = '0';
    } else {
        while (num > 0u) {
            buf[--i] = (char)('0' + (num % 10u));
            num /= 10u;
        }
    }
    vga_draw_string(x, y, &buf[i], color);
}

/* ── Keyboard functions ─────────────────────────────────────────────────── */

void keyboard_init(void) { /* no-op */ }

int keyboard_get_key(void) {
    return vicell_poll_key();
}

/* ── Timer functions ────────────────────────────────────────────────────── */

void timer_init(void) { /* no-op */ }

uint32_t timer_get_ticks(void) {
    return vicell_get_ticks_ms();
}

/* ── Speaker stubs ──────────────────────────────────────────────────────── */

void speaker_init(void)          { /* no audio hardware */ }
void speaker_music_start(void)   { }
void speaker_music_stop(void)    { }
void speaker_music_tick(void)    { }
/* Some Tetris-OS builds also call speaker_beep / speaker_off: */
void speaker_beep(uint32_t freq) { (void)freq; }
void speaker_off(void)           { }

/* ── x86 kernel bootstrap stubs ─────────────────────────────────────────── */
/* These may be referenced from main.c in a bare-metal x86 build.
 * On ViCell they are no-ops since the kernel already handles all hardware. */
void gdt_init(void)  { }
void idt_init(void)  { }
void pic_init(void)  { }
void pic_eoi(uint8_t irq) { (void)irq; }
void irq_install_handler(int irq, void (*handler)(void)) { (void)irq; (void)handler; }

/* ── Game entry point ───────────────────────────────────────────────────── */
/*
 * Called from Rust main() after the compositor surface is ready.
 *
 * VERIFY AFTER CLONING: check tetris.h for the actual game entry function name.
 *   - If tetris.c exports tetris_run()  → this works as-is.
 *   - If tetris.c exports tetris_main() → change the call below.
 *   - If the loop lives in main.c        → see the alternative in build.rs.
 */
extern void tetris_run(void);   /* declared in tetris.c / tetris.h */

void tetris_cell_run(void) {
    tetris_run();
}
