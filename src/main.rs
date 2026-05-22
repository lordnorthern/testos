// ============================================================================
// BARE-METAL RUST OS KERNEL — main.rs
// ============================================================================
//
// This is the heart of our kernel. When the Raspberry Pi 2 powers on, the GPU
// runs some ROM code, loads a file called "kernel7.img" from the SD card into
// RAM at address 0x8000, and then hands control to the CPU. Eventually that
// chain of boot code calls `kernel_main()` below — and this file takes over.
//
// Nothing from the Rust standard library is available here. There is no heap
// allocator, no threads, no file system, no OS underneath us. We ARE the OS.
// Every byte of every string we print, we have to send ourselves over hardware.
// ============================================================================


// ----------------------------------------------------------------------------
// ATTRIBUTE: #![no_std]
// ----------------------------------------------------------------------------
// Normally Rust programs link against `std`, the standard library. std gives
// you things like Vec, String, File, threads, and println!. But std itself
// depends on an operating system being underneath it — it calls OS system
// calls to allocate memory, open files, etc.
//
// Since WE are the OS, there is nothing underneath us to call. So we tell
// Rust: "don't link std". We still get `core`, which is the OS-independent
// subset of Rust (things like Option, Result, basic math, iterators).
#![no_std]

// ----------------------------------------------------------------------------
// ATTRIBUTE: #![no_main]
// ----------------------------------------------------------------------------
// Normally a Rust program has a `fn main()` entry point. But that `main` is
// actually called BY the Rust runtime (which sets up stack unwinding, calls
// global constructors, etc.) which itself is called by a C runtime (crt0)
// which is called by the OS loader.
//
// We don't have any of that scaffolding. Our "main" will be called directly
// by our boot assembly code with a raw jump, so Rust must not expect to set
// up or tear down anything around it. `#![no_main]` tells Rust: "I'll define
// my own entry point — don't generate a default one."
#![no_main]


// ----------------------------------------------------------------------------
// MODULE DECLARATIONS
// ----------------------------------------------------------------------------
// `mod foo;` tells Rust to look for either `src/foo.rs` or `src/foo/mod.rs`
// and compile it as a sub-module. Think of it like `#include` in C, but with
// proper namespacing — symbols from `uart` are accessed as `uart::init()`.

// boot.rs — very early startup: sets up the stack pointer in assembly so
// that Rust code can run at all. Typically just a `.S` file linked in.
mod boot;

// console.rs — a unified output abstraction. Writes to both UART and the
// framebuffer at the same time, so text appears on both serial and HDMI.
mod console;

// fb.rs — framebuffer driver. Talks to the VideoCore GPU via the mailbox
// to get a chunk of memory that maps directly to pixels on screen.
mod fb;

// font.rs — a bitmap font (each character is a grid of 1/0 bits). Used by
// the framebuffer code to draw text as pixels on the HDMI display.
mod font;

// mailbox.rs — the Raspberry Pi's mailbox is how the ARM CPU asks the GPU
// to do things (like allocate a framebuffer). It's a shared memory region
// with a simple protocol: write a message, ring a bell, wait for a reply.
mod mailbox;

// uart.rs — Universal Asynchronous Receiver/Transmitter driver. The RPi2
// has a "Mini UART" wired to the GPIO pins 14/15. When you plug in a USB
// serial adapter you can type commands and see output on your laptop.
mod uart;


// ----------------------------------------------------------------------------
// IMPORTS
// ----------------------------------------------------------------------------
// Even without `std`, the `core` crate gives us essential traits.

// `Write` is a trait (like an interface) that lets you use the `write!()`
// macro. We implement it on our `console::Writer` so we can format strings
// with `{}` placeholders just like `println!` — but over our own hardware.
use core::fmt::Write;

// `PanicInfo` carries information about what went wrong when Rust panics
// (e.g., an out-of-bounds array access, or an explicit `panic!("message")`).
// We need it for the panic handler below.
use core::panic::PanicInfo;


// ----------------------------------------------------------------------------
// PANIC HANDLER
// ----------------------------------------------------------------------------
// In normal Rust, when a panic happens the standard library prints a message
// and unwinds the stack (calls destructors, etc.). We have none of that.
//
// `#[panic_handler]` marks this function as THE thing Rust calls when any
// panic occurs anywhere in our kernel. We must define exactly one.
//
// The return type `-> !` means "diverging" — this function never returns.
// That makes sense: after a kernel panic, there's nowhere safe to return to.
// We print the error and spin forever (halting the core).
#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    // `write!` uses the `Write` trait. `console::Writer` is a zero-sized
    // type (no data, just methods) that implements `Write` by forwarding to
    // both UART and the framebuffer. The `let _ =` discards any error — if
    // the console itself is broken, there's nothing more we can do.
    let _ = write!(console::Writer, "\r\n[PANIC] {}\r\n", info);

    // Spin forever. On a real system you might also halt other CPU cores,
    // disable interrupts, or blink an LED. For now, looping is enough.
    loop {}
}


// ----------------------------------------------------------------------------
// KERNEL ENTRY POINT
// ----------------------------------------------------------------------------
// This is where execution begins (after the boot assembly sets up the stack).
//
// `#[no_mangle]` — Rust normally "mangles" function names (appends type info
// to make them unique). That would turn `kernel_main` into something like
// `_ZN2os11kernel_mainE`. Our linker script and boot code need to jump to the
// *exact* symbol name `kernel_main`, so we tell Rust to leave the name alone.
//
// `pub extern "C"` — `extern "C"` makes the function use the C calling
// convention (arguments in specific registers, return value in r0, etc.).
// Our assembly boot stub calls this as if it were a C function, so the
// calling conventions must match. `pub` makes it visible to the linker.
//
// `-> !` — again, diverging. The kernel never "returns" anywhere. It loops
// forever handling input. If it ever returned, the CPU would execute whatever
// bytes come next in memory — undefined behavior at the hardware level.
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // ---- Hardware initialization ----
    //
    // We must initialize hardware before we can use it. Order matters:
    // UART first so we have some way to see debug output even if fb fails.

    // Set up the Mini UART: configure baud rate (115200), enable TX/RX,
    // map GPIO pins 14 and 15 to UART function. After this we can send/recv
    // bytes over the serial port.
    uart::init();

    // Try to set up an HDMI framebuffer via the GPU mailbox. This might fail
    // if there's no display connected or the mailbox is busy, so it's
    // "best-effort". The comment on this line explains the fallback behavior.
    fb::init(); // best-effort; console falls back to UART-only if this fails


    // ---- Splash banner ----
    //
    // `console::puts` sends a null-terminated-style byte slice to ALL outputs
    // (both UART and the framebuffer if it's up). `\r\n` is carriage-return +
    // newline — serial terminals need *both* characters to move to the start
    // of the next line (just `\n` leaves the cursor at the same column).
    console::puts("\r\n");
    console::puts("+==============================+\r\n");
    console::puts("|   RPi2 Bare Metal Shell      |\r\n");
    console::puts("|   Rust  *  no OS  *  ARMv7   |\r\n");
    console::puts("+==============================+\r\n");
    console::puts("\r\nType 'help' for commands.\r\n\r\n");


    // ---- Input buffer ----
    //
    // We need somewhere to store the characters the user types before they
    // press Enter. We use a fixed-size array on the stack — no heap, no Vec.
    //
    // 128 bytes is plenty for a simple command line. If the user types more,
    // we just stop accepting characters (see the match arm below).
    let mut buf = [0u8; 128]; // 128-byte line buffer, initialized to zeros

    // `len` tracks how many valid bytes are currently in `buf`. We use a
    // slice `&buf[..len]` to refer to just the filled portion.
    let mut len = 0usize;


    // Show the initial prompt ("rpi2> ") so the user knows we're ready.
    print_prompt();


    // ---- Main shell loop ----
    //
    // This is the heart of the shell. We spin here forever, reading one
    // character at a time from UART, updating the buffer, and dispatching
    // commands when the user presses Enter.
    //
    // On a real OS this would be a process managed by a scheduler. Here it's
    // literally just a `loop {}` — the kernel IS the shell.
    loop {
        // `uart::getc()` blocks until a byte arrives. "Blocks" means: it
        // keeps reading the UART status register in a tight loop until the
        // "data ready" bit is set, then reads the byte. No interrupts, no
        // sleep — this is called "polling" or "busy-waiting".
        let c = uart::getc();

        // Pattern-match on the byte value. In Rust, `match` on a u8 works
        // like a switch statement but exhaustive — we must handle every case.
        match c {
            // ---- Enter / Return ----
            // `\r` = carriage return (0x0D), `\n` = newline (0x0A).
            // Different terminals send different ones; we accept both.
            b'\r' | b'\n' => {
                // Move to the next line on the terminal.
                console::puts("\r\n");

                // Only bother running a command if something was typed.
                if len > 0 {
                    // Pass a slice of just the typed bytes to the command
                    // dispatcher. `&buf[..len]` is "a reference to the first
                    // `len` bytes of buf" — no copy, no allocation.
                    run_command(&buf[..len]);

                    // Reset the length to 0, effectively "clearing" the
                    // buffer. The old bytes are still in memory but we'll
                    // overwrite them as the user types the next command.
                    len = 0;
                }

                // Print the prompt again, ready for the next command.
                print_prompt();
            }

            // ---- Backspace / Delete ----
            // 0x08 = ASCII BS (backspace), 0x7F = ASCII DEL. Different
            // terminals send one or the other when you press Backspace.
            0x08 | 0x7F => {
                // Only erase if there's something to erase.
                if len > 0 {
                    len -= 1; // Remove the last character from our buffer.

                    // The sequence "\x08 \x08" does three things on the terminal:
                    //   \x08 — move cursor left one column
                    //   ' '  — overwrite the character with a space (erase it visually)
                    //   \x08 — move cursor left again (so the cursor is back where it was)
                    // This is the classic terminal backspace trick.
                    console::puts("\x08 \x08");
                }
            }

            // ---- Printable ASCII characters ----
            // 0x20 = space, 0x7E = '~'. Everything in this range is a
            // character we can display and store. The `..=` syntax is an
            // inclusive range pattern.
            0x20..=0x7E => {
                // Guard against buffer overflow — if the line is already
                // at max length, silently ignore the character. A fancier
                // shell might beep (send 0x07, the BEL character).
                if len < buf.len() {
                    buf[len] = c; // Store the byte in the buffer.
                    len += 1;     // Advance the write position.

                    // Echo the character back so the user sees what they
                    // typed. UART doesn't echo automatically — if we didn't
                    // do this, the terminal would appear blank as you type.
                    console::putc(c);
                }
            }

            // ---- Everything else ----
            // Control characters, extended ASCII, etc. We just ignore them.
            // `_` is Rust's wildcard pattern — matches anything not caught above.
            _ => {}
        }
    }
    // The loop never exits, so `kernel_main` satisfies the `-> !` contract.
}


// ----------------------------------------------------------------------------
// PRINT PROMPT
// ----------------------------------------------------------------------------
// A tiny helper so the prompt string is defined in exactly one place. If we
// ever want to change it (e.g., show the current "directory"), we only change
// it here.
fn print_prompt() {
    console::puts("rpi2> ");
}


// ----------------------------------------------------------------------------
// COMMAND DISPATCHER
// ----------------------------------------------------------------------------
// Called with a byte slice containing exactly what the user typed (no trailing
// newline). We split it on the first space to get a verb and optional args,
// then dispatch to the right handler.
//
// Everything here is done with byte slices (`&[u8]`) rather than `&str`
// because we have no guarantee the input is valid UTF-8 (it comes raw from
// UART). We compare against byte string literals like `b"help"`.
fn run_command(input: &[u8]) {
    // Split the input into (verb, args) on the first space character.
    //
    // `input.iter().position(|&b| b == b' ')` scans the slice and returns
    // `Some(index)` of the first space, or `None` if there is no space.
    //
    // `match` on the Option lets us handle both cases:
    //   Some(i) → verb is everything before index i, args is everything after
    //   None    → no space found, the whole input is the verb, args is empty
    //
    // `&b""[..]` is the empty byte slice — a pointer to a zero-length region.
    let (verb, args) = match input.iter().position(|&b| b == b' ') {
        Some(i) => (&input[..i], &input[i + 1..]),
        None    => (input, &b""[..]),
    };

    // Match on the verb byte slice. Rust lets us match byte string literals
    // directly against `&[u8]` slices — very convenient.
    match verb {
        // ---- help ----
        // Print a list of all known commands.
        b"help" => {
            console::puts("Commands:\r\n");
            console::puts("  help            show this message\r\n");
            console::puts("  echo <text>     print text\r\n");
            console::puts("  clear           clear the screen\r\n");
            console::puts("  info            hardware info\r\n");
            console::puts("  hex <addr>      read 32-bit MMIO register\r\n");
        }

        // ---- echo ----
        // Print whatever the user typed after "echo ". Since `args` is a
        // byte slice we iterate byte-by-byte and write each one individually.
        b"echo" => {
            for &b in args { console::putc(b); }
            console::puts("\r\n");
        }

        // ---- clear ----
        // Clear the screen on both outputs.
        b"clear" => {
            // ANSI escape sequence: ESC [ 2 J = erase entire screen,
            //                       ESC [ H   = move cursor to top-left.
            // Serial terminals (like minicom, PuTTY) understand these codes.
            uart::puts("\x1b[2J\x1b[H"); // ANSI clear for serial terminal

            // The framebuffer doesn't understand ANSI codes — it's just raw
            // pixels. We call fb::clear() to fill every pixel with black.
            fb::clear();                 // pixel clear for HDMI
        }

        // ---- info ----
        // Print basic hardware information about this board.
        b"info" => {
            // These are hard-coded strings because we know exactly what
            // hardware we're targeting: the Raspberry Pi 2 (BCM2836 SoC).
            console::puts("CPU:    ARM Cortex-A7 (BCM2836)\r\n");
            console::puts("Arch:   ARMv7-A, 32-bit, SVC mode\r\n");
            console::puts("UART:   Mini UART, 115200 8N1\r\n"); // 115200 baud, 8 data bits, No parity, 1 stop bit
            console::puts("HDMI:   1024x768 32bpp framebuffer\r\n"); // 32 bits per pixel = ARGB

            // `write!` uses the `Write` trait to format a string with `{}`.
            // `fb::is_ready()` returns a bool; the ternary-style `if` in Rust
            // is an expression that evaluates to one of the two string arms.
            let _ = write!(console::Writer, "FB:     {}\r\n",
                if fb::is_ready() { "active" } else { "unavailable" });
        }

        // ---- hex ----
        // Read a 32-bit value from a memory-mapped I/O address and print it.
        // This is incredibly useful for OS development — you can inspect any
        // hardware register directly from the shell without rebooting.
        //
        // Example: `hex 0x3F215060` reads the Mini UART's line status register.
        b"hex" => {
            if args.is_empty() {
                // No address given — print usage hint.
                console::puts("usage: hex <address>\r\n");
            } else {
                // Try to parse the argument as a hexadecimal number.
                // `parse_hex` returns Option<usize>: Some(addr) on success,
                // None if the string wasn't valid hex.
                match parse_hex(args) {
                    Some(addr) => {
                        // `unsafe` is required here because we're doing raw
                        // pointer dereference. Rust normally prevents this to
                        // avoid crashes, but reading hardware registers is
                        // inherently unsafe — we must promise the compiler
                        // that we know what we're doing.
                        //
                        // `addr as *const u32` casts our usize address to a
                        // typed raw pointer to a 32-bit unsigned integer.
                        //
                        // `read_volatile` tells the compiler: "always actually
                        // read from this address — do NOT cache the result or
                        // optimize this away." Hardware registers change value
                        // without the CPU writing to them, so the compiler must
                        // not assume the value is constant between reads.
                        let val = unsafe { core::ptr::read_volatile(addr as *const u32) };

                        // Print both the address and value in hex with `{:08x}`
                        // (lowercase hex, padded to 8 digits with leading zeros).
                        let _ = write!(console::Writer, "0x{:08x} = 0x{:08x}\r\n", addr, val);
                    }
                    None => console::puts("invalid hex address\r\n"),
                }
            }
        }

        // ---- unknown command ----
        // The user typed something we don't recognise. Echo back what they
        // typed so they know what was received (helpful if there's a typo or
        // invisible character).
        _ => {
            console::puts("unknown command: ");
            for &b in verb { console::putc(b); }
            console::puts("\r\n");
        }
    }
}


// ----------------------------------------------------------------------------
// HEX STRING PARSER
// ----------------------------------------------------------------------------
// Converts a byte slice like b"0x3F200000" or b"3f200000" into a `usize`.
// Returns `None` if any character isn't a valid hex digit.
//
// We can't use the standard library's `usize::from_str_radix` here because
// that operates on `&str` (UTF-8 strings) and requires std. We do it manually.
fn parse_hex(s: &[u8]) -> Option<usize> {
    // Strip a leading "0x" or "0X" prefix if present.
    // `s.starts_with(b"0x")` checks if the first two bytes are '0' and 'x'.
    // `&s[2..]` is the slice starting at byte index 2 (skipping the prefix).
    let s = if s.starts_with(b"0x") || s.starts_with(b"0X") { &s[2..] } else { s };

    // Reject empty input (e.g., the user typed just "0x" with nothing after).
    if s.is_empty() { return None; }

    let mut val: usize = 0;

    // Process each byte of the hex string one nibble at a time.
    // A "nibble" is 4 bits — one hex digit represents exactly one nibble.
    for &b in s {
        // Convert the ASCII character to its numeric nibble value (0–15).
        // Return None immediately if the character isn't valid hex.
        let nibble = match b {
            b'0'..=b'9' => (b - b'0') as usize,       // '0'→0, '9'→9
            b'a'..=b'f' => (b - b'a' + 10) as usize,  // 'a'→10, 'f'→15
            b'A'..=b'F' => (b - b'A' + 10) as usize,  // 'A'→10, 'F'→15
            _ => return None, // Not a hex digit — bail out.
        };

        // Shift the accumulator left by 4 bits (make room for the new nibble)
        // then add the new nibble in the low 4 bits.
        //
        // `checked_shl(4)` returns None if the shift would overflow (i.e., the
        // number is too large to fit in a usize). Same for `checked_add`.
        // Using `?` here propagates None upward — if either operation would
        // overflow, `parse_hex` returns None instead of silently wrapping.
        val = val.checked_shl(4)?.checked_add(nibble)?;
    }

    Some(val)
}
