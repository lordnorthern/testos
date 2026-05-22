// HDMI framebuffer via VideoCore mailbox.
// Resolution: 1024x768, 32bpp (0x00RRGGBB).
// Text rendered with the 8x8 bitmap font; scrolls when the bottom is reached.

use crate::{font, mailbox};
use core::ptr::write_volatile;

static mut READY:  bool   = false;
static mut BASE:   *mut u32 = core::ptr::null_mut();
static mut WIDTH:  u32    = 0;
static mut HEIGHT: u32    = 0;
static mut PITCH:  u32    = 0; // bytes per row
static mut CX:     u32    = 0; // cursor x in pixels
static mut CY:     u32    = 0; // cursor y in pixels

const CW: u32 = font::W as u32;
const CH: u32 = font::H as u32;

const FG: u32 = 0x00E0E0E0; // light grey text
const BG: u32 = 0x00000000; // black background

pub fn init() -> bool {
    let mut m = mailbox::Msg::new();

    // Buffer size = 35 words × 4 bytes = 140
    m.0[0]  = 140;
    m.0[1]  = 0;           // request

    m.0[2]  = 0x00048003;  // set physical width/height
    m.0[3]  = 8; m.0[4] = 0;
    m.0[5]  = 1024;
    m.0[6]  = 768;

    m.0[7]  = 0x00048004;  // set virtual width/height
    m.0[8]  = 8; m.0[9] = 0;
    m.0[10] = 1024;
    m.0[11] = 768;

    m.0[12] = 0x00048009;  // set virtual offset
    m.0[13] = 8; m.0[14] = 0;
    m.0[15] = 0;
    m.0[16] = 0;

    m.0[17] = 0x00048005;  // set depth (bpp)
    m.0[18] = 4; m.0[19] = 0;
    m.0[20] = 32;

    m.0[21] = 0x00048006;  // set pixel order: 1 = RGB
    m.0[22] = 4; m.0[23] = 0;
    m.0[24] = 1;

    m.0[25] = 0x00040001;  // allocate framebuffer (alignment in, base+size out)
    m.0[26] = 8; m.0[27] = 0;
    m.0[28] = 4096;        // -> becomes bus address of framebuffer
    m.0[29] = 0;           // -> becomes size in bytes

    m.0[30] = 0x00040008;  // get pitch
    m.0[31] = 4; m.0[32] = 0;
    m.0[33] = 0;           // -> becomes bytes per row

    m.0[34] = 0;           // end tag

    if !mailbox::call(&mut m) {
        return false;
    }

    // GPU returns a bus address; strip the top bits to get the ARM physical address
    let base = m.0[28] & 0x3FFF_FFFF;
    if base == 0 || m.0[33] == 0 {
        return false;
    }

    unsafe {
        BASE   = base as *mut u32;
        WIDTH  = m.0[5];
        HEIGHT = m.0[6];
        PITCH  = m.0[33];
        CX     = 0;
        CY     = 0;
        READY  = true;
    }

    true
}

pub fn is_ready() -> bool {
    unsafe { READY }
}

pub fn clear() {
    unsafe {
        if !READY { return; }
        core::ptr::write_bytes(BASE as *mut u8, 0, (HEIGHT * PITCH) as usize);
        CX = 0;
        CY = 0;
    }
}

fn pset(x: u32, y: u32, color: u32) {
    unsafe {
        write_volatile(BASE.add((y * (PITCH / 4) + x) as usize), color);
    }
}

fn draw_char(c: u8, px: u32, py: u32) {
    let glyph = &font::FONT[if (0x20..=0x7E).contains(&c) { (c - 0x20) as usize } else { 0 }];
    for row in 0..CH {
        let bits = glyph[row as usize];
        for col in 0..CW {
            pset(px + col, py + row, if bits & (0x80 >> col) != 0 { FG } else { BG });
        }
    }
}

fn scroll() {
    unsafe {
        let row_bytes = (CH * PITCH) as usize;
        let total     = (HEIGHT * PITCH) as usize;
        let fb = BASE as *mut u8;
        core::ptr::copy(fb.add(row_bytes), fb, total - row_bytes);
        core::ptr::write_bytes(fb.add(total - row_bytes), 0, row_bytes);
        CY -= CH;
    }
}

fn advance() {
    unsafe {
        CX = 0;
        CY += CH;
        if CY + CH > HEIGHT { scroll(); }
    }
}

pub fn putc(c: u8) {
    unsafe {
        if !READY { return; }
        match c {
            b'\r' => { CX = 0; }
            b'\n' => { advance(); }
            0x08 | 0x7F => {
                if CX >= CW {
                    CX -= CW;
                    for row in 0..CH {
                        for col in 0..CW { pset(CX + col, CY + row, BG); }
                    }
                }
            }
            0x20..=0x7E => {
                draw_char(c, CX, CY);
                CX += CW;
                if CX + CW > WIDTH { advance(); }
            }
            _ => {}
        }
    }
}

/// Only renders printable ASCII; silently skips UTF-8 continuation bytes
/// so multi-byte characters in UART strings don't corrupt the display.
pub fn puts(s: &str) {
    for b in s.bytes() {
        if b < 0x80 { putc(b); }
    }
}
