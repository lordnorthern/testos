// BCM2836 (RPi2) Mini UART (AUX UART1) driver.
// GPIO 14 = TXD1 (ALT5), GPIO 15 = RXD1 (ALT5).
// Baud: 250 MHz core clock / (8 * 115200) - 1 = 270.
// config.txt must set core_freq=250 to keep this stable.

use core::fmt;
use core::ptr::{read_volatile, write_volatile};

const MMIO_BASE: usize = 0x3F00_0000;
const GPIO_BASE: usize = MMIO_BASE + 0x0020_0000;
const AUX_BASE:  usize = MMIO_BASE + 0x0021_5000;

const GPFSEL1:    *mut u32 = (GPIO_BASE + 0x04) as *mut u32;
const GPPUD:      *mut u32 = (GPIO_BASE + 0x94) as *mut u32;
const GPPUDCLK0:  *mut u32 = (GPIO_BASE + 0x98) as *mut u32;

const AUX_ENABLES: *mut u32 = (AUX_BASE + 0x04) as *mut u32;
const AUX_MU_IO:   *mut u32 = (AUX_BASE + 0x40) as *mut u32;
const AUX_MU_IER:  *mut u32 = (AUX_BASE + 0x44) as *mut u32;
const AUX_MU_IIR:  *mut u32 = (AUX_BASE + 0x48) as *mut u32;
const AUX_MU_LCR:  *mut u32 = (AUX_BASE + 0x4C) as *mut u32;
const AUX_MU_MCR:  *mut u32 = (AUX_BASE + 0x50) as *mut u32;
const AUX_MU_LSR:  *mut u32 = (AUX_BASE + 0x54) as *mut u32;
const AUX_MU_CNTL: *mut u32 = (AUX_BASE + 0x60) as *mut u32;
const AUX_MU_BAUD: *mut u32 = (AUX_BASE + 0x68) as *mut u32;

fn delay(cycles: u32) {
    for _ in 0..cycles {
        unsafe { core::arch::asm!("nop") };
    }
}

pub fn init() {
    unsafe {
        // Enable Mini UART and disable TX/RX while configuring
        write_volatile(AUX_ENABLES, 1);
        write_volatile(AUX_MU_CNTL, 0);
        write_volatile(AUX_MU_IER,  0);
        write_volatile(AUX_MU_LCR,  3);    // 8-bit
        write_volatile(AUX_MU_MCR,  0);
        write_volatile(AUX_MU_IIR,  0xC6); // clear FIFOs
        write_volatile(AUX_MU_BAUD, 270);  // 115200 @ 250 MHz

        // GPIO 14 & 15 → ALT5 (Mini UART TXD1/RXD1)
        // GPFSEL1 bits [14:12]=GPIO14, [17:15]=GPIO15; ALT5=0b010
        let mut sel = read_volatile(GPFSEL1);
        sel &= !((0b111 << 12) | (0b111 << 15));
        sel |=   (0b010 << 12) | (0b010 << 15);
        write_volatile(GPFSEL1, sel);

        // BCM2835/2836 pull-up/down: write value → delay → clock pins → delay → clear
        write_volatile(GPPUD, 0); // no pull
        delay(150);
        write_volatile(GPPUDCLK0, (1 << 14) | (1 << 15));
        delay(150);
        write_volatile(GPPUD, 0);
        write_volatile(GPPUDCLK0, 0);

        // Enable TX and RX
        write_volatile(AUX_MU_CNTL, 3);
    }
}

pub fn putc(c: u8) {
    unsafe {
        while read_volatile(AUX_MU_LSR) & 0x20 == 0 {}
        write_volatile(AUX_MU_IO, c as u32);
    }
}

pub fn getc() -> u8 {
    unsafe {
        while read_volatile(AUX_MU_LSR) & 0x01 == 0 {}
        (read_volatile(AUX_MU_IO) & 0xFF) as u8
    }
}

pub fn puts(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            putc(b'\r');
        }
        putc(b);
    }
}

pub struct Writer;

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        puts(s);
        Ok(())
    }
}
