// Dual-output console: every write goes to UART and (if ready) the framebuffer.
// Input still comes exclusively from UART.

use core::fmt;
use crate::{fb, uart};

pub fn putc(c: u8) {
    uart::putc(c);
    fb::putc(c);
}

pub fn puts(s: &str) {
    uart::puts(s);
    fb::puts(s);
}

pub struct Writer;

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        puts(s);
        Ok(())
    }
}
