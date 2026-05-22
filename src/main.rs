#![no_std]
#![no_main]

mod boot;
mod console;
mod fb;
mod font;
mod mailbox;
mod uart;

use core::fmt::Write;
use core::panic::PanicInfo;

#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    let _ = write!(console::Writer, "\r\n[PANIC] {}\r\n", info);
    loop {}
}

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    uart::init();
    fb::init(); // best-effort; console falls back to UART-only if this fails

    console::puts("\r\n");
    console::puts("+==============================+\r\n");
    console::puts("|   RPi2 Bare Metal Shell      |\r\n");
    console::puts("|   Rust  *  no OS  *  ARMv7   |\r\n");
    console::puts("+==============================+\r\n");
    console::puts("\r\nType 'help' for commands.\r\n\r\n");

    let mut buf = [0u8; 128];
    let mut len = 0usize;

    print_prompt();

    loop {
        let c = uart::getc();
        match c {
            b'\r' | b'\n' => {
                console::puts("\r\n");
                if len > 0 {
                    run_command(&buf[..len]);
                    len = 0;
                }
                print_prompt();
            }
            0x08 | 0x7F => {
                if len > 0 {
                    len -= 1;
                    console::puts("\x08 \x08");
                }
            }
            0x20..=0x7E => {
                if len < buf.len() {
                    buf[len] = c;
                    len += 1;
                    console::putc(c);
                }
            }
            _ => {}
        }
    }
}

fn print_prompt() {
    console::puts("rpi2> ");
}

fn run_command(input: &[u8]) {
    let (verb, args) = match input.iter().position(|&b| b == b' ') {
        Some(i) => (&input[..i], &input[i + 1..]),
        None    => (input, &b""[..]),
    };

    match verb {
        b"help" => {
            console::puts("Commands:\r\n");
            console::puts("  help            show this message\r\n");
            console::puts("  echo <text>     print text\r\n");
            console::puts("  clear           clear the screen\r\n");
            console::puts("  info            hardware info\r\n");
            console::puts("  hex <addr>      read 32-bit MMIO register\r\n");
        }
        b"echo" => {
            for &b in args { console::putc(b); }
            console::puts("\r\n");
        }
        b"clear" => {
            uart::puts("\x1b[2J\x1b[H"); // ANSI clear for serial terminal
            fb::clear();                 // pixel clear for HDMI
        }
        b"info" => {
            console::puts("CPU:    ARM Cortex-A7 (BCM2836)\r\n");
            console::puts("Arch:   ARMv7-A, 32-bit, SVC mode\r\n");
            console::puts("UART:   Mini UART, 115200 8N1\r\n");
            console::puts("HDMI:   1024x768 32bpp framebuffer\r\n");
            let _ = write!(console::Writer, "FB:     {}\r\n",
                if fb::is_ready() { "active" } else { "unavailable" });
        }
        b"hex" => {
            if args.is_empty() {
                console::puts("usage: hex <address>\r\n");
            } else {
                match parse_hex(args) {
                    Some(addr) => {
                        let val = unsafe { core::ptr::read_volatile(addr as *const u32) };
                        let _ = write!(console::Writer, "0x{:08x} = 0x{:08x}\r\n", addr, val);
                    }
                    None => console::puts("invalid hex address\r\n"),
                }
            }
        }
        _ => {
            console::puts("unknown command: ");
            for &b in verb { console::putc(b); }
            console::puts("\r\n");
        }
    }
}

fn parse_hex(s: &[u8]) -> Option<usize> {
    let s = if s.starts_with(b"0x") || s.starts_with(b"0X") { &s[2..] } else { s };
    if s.is_empty() { return None; }
    let mut val: usize = 0;
    for &b in s {
        let nibble = match b {
            b'0'..=b'9' => (b - b'0') as usize,
            b'a'..=b'f' => (b - b'a' + 10) as usize,
            b'A'..=b'F' => (b - b'A' + 10) as usize,
            _ => return None,
        };
        val = val.checked_shl(4)?.checked_add(nibble)?;
    }
    Some(val)
}
