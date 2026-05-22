# RPi2 Bare Metal Rust — Developer Guide

A deep walkthrough of every file in this project: what it does, why it exists,
how the hardware works, and what you need to know to take it further.

---

## Table of Contents

1. [What This Is](#1-what-this-is)
2. [The Raspberry Pi Boot Chain](#2-the-raspberry-pi-boot-chain)
3. [Project Layout](#3-project-layout)
4. [The Toolchain — Rust on Bare Metal](#4-the-toolchain--rust-on-bare-metal)
5. [The Linker Script — `linker.ld`](#5-the-linker-script--linkerld)
6. [Boot Assembly — `src/boot.rs`](#6-boot-assembly--srcbootrs)
7. [UART Driver — `src/uart.rs`](#7-uart-driver--srcuartrs)
8. [VideoCore Mailbox — `src/mailbox.rs`](#8-videocore-mailbox--srcmailboxrs)
9. [Framebuffer Driver — `src/fb.rs`](#9-framebuffer-driver--srcfbrs)
10. [Bitmap Font — `src/font.rs`](#10-bitmap-font--srcfontrs)
11. [Console Abstraction — `src/console.rs`](#11-console-abstraction--srcconsole rs)
12. [The Kernel Shell — `src/main.rs`](#12-the-kernel-shell--srcmainrs)
13. [The SD Card — `config.txt`](#13-the-sd-card--configtxt)
14. [Building and Flashing](#14-building-and-flashing)
15. [Memory Map](#15-memory-map)
16. [MMIO Register Reference](#16-mmio-register-reference)
17. [How to Extend This](#17-how-to-extend-this)

---

## 1. What This Is

This is a **bare-metal kernel** written in Rust that runs directly on a
Raspberry Pi 2 (BCM2836, ARM Cortex-A7). There is no operating system. The
firmware hands the CPU directly to this code at address `0x8000`, and from that
point on, this code owns the machine entirely.

"Bare metal" means:
- No Linux, no RTOS, no libc, no heap allocator.
- No system calls. You talk to hardware by reading and writing memory-mapped
  registers at specific physical addresses.
- No interrupts are enabled. Everything is polled (spin-waiting for hardware
  to be ready).
- One CPU core runs; the other three are parked in a low-power loop.

The result is a serial/HDMI shell with five commands (`help`, `echo`, `clear`,
`info`, `hex`). It is intentionally minimal — a launchpad you can extend in
any direction.

---

## 2. The Raspberry Pi Boot Chain

Understanding the boot chain is essential. There are multiple layers before
your code runs.

### Step 1 — The GPU wakes up first

The BCM2836 SoC contains both an ARM CPU and a VideoCore IV GPU. When power is
applied, **the GPU boots first** from an on-chip ROM. The ARM CPU is held in
reset.

The GPU ROM looks for a FAT32 partition on the SD card and loads `bootcode.bin`
into GPU L2 cache, then executes it.

### Step 2 — `bootcode.bin`

This is the second-stage GPU bootloader. It initialises SDRAM, then loads
`start.elf` from the SD card.

### Step 3 — `start.elf`

This is the main GPU firmware. It reads `config.txt` for configuration, then
loads the kernel image. For a 32-bit ARMv7 kernel the filename is `kernel7.img`
(you tell it which name via `kernel=kernel7.img` in `config.txt`).

`start.elf` loads `kernel7.img` to physical address **`0x8000`** (32 KiB into
RAM), releases the ARM CPU from reset, and sets the program counter to `0x8000`.

### Step 4 — Your code runs

The first instruction executed is `_start` in `src/boot.rs`. The ARM CPU is now
in SVC or HYP mode (depending on firmware version), with no stack set up, BSS
not zeroed, and the other three CPU cores also waking up simultaneously.

Your boot code must handle all of that before Rust code can safely run.

### Why `0x8000`?

The first 32 KiB of RAM (`0x0000`–`0x7FFF`) is reserved by the firmware for
its own data (ATAGS, device tree blob, etc.). The GPU writes information there
that some kernels read; this project ignores it. Your kernel occupies `0x8000`
onwards.

---

## 3. Project Layout

```
os/
├── .cargo/
│   └── config.toml          # Cross-compilation settings for Cargo
├── src/
│   ├── boot.rs              # ARM32 assembly — first code to run
│   ├── uart.rs              # Mini UART (serial) driver
│   ├── mailbox.rs           # GPU mailbox protocol
│   ├── fb.rs                # HDMI framebuffer driver
│   ├── font.rs              # 8×8 bitmap font data
│   ├── console.rs           # Dual-output (UART + HDMI) console
│   └── main.rs              # kernel_main — the interactive shell
├── Cargo.toml               # Package metadata and build profiles
├── linker.ld                # Linker script — controls memory layout
├── rust-toolchain.toml      # Pins the Rust nightly version
└── config.txt               # Raspberry Pi firmware configuration
```

The SD card (the files the Pi actually loads) contains:
```
SD card FAT32 root/
├── bootcode.bin             # GPU 2nd-stage bootloader (from RPi firmware repo)
├── start.elf                # GPU main firmware   (from RPi firmware repo)
├── fixup.dat                # GPU memory split config (from RPi firmware repo)
├── config.txt               # Your 2-line config file
└── kernel7.img              # Your compiled binary
```

---

## 4. The Toolchain — Rust on Bare Metal

### `rust-toolchain.toml`

```toml
[toolchain]
channel = "nightly"
components = ["rust-src"]
targets = ["armv7a-none-eabi"]
```

**Why nightly?** The `build-std` feature (compiling `core` from source as part
of your build) is only available on nightly Rust. Stable Rust ships pre-built
copies of `core` for tier-1 and tier-2 targets. `armv7a-none-eabi` is a tier-3
target — no pre-built `core` exists — so you must build it yourself.

**`rust-src` component** provides the Rust standard library source code that
`build-std` needs to compile `core`.

**The target `armv7a-none-eabi`** means:
- `armv7a` — ARMv7-A architecture (Cortex-A7 supports this)
- `none` — no operating system
- `eabi` — Embedded ABI (calling convention and ABI for bare-metal ARM)

### `.cargo/config.toml`

```toml
[build]
target = "armv7a-none-eabi"

[unstable]
build-std = ["core"]
build-std-features = ["compiler-builtins-mem"]

[target.armv7a-none-eabi]
rustflags = [
    "-C", "link-arg=-Tlinker.ld",
    "-C", "link-arg=--build-id=none",
    "-C", "target-cpu=cortex-a7",
]
```

**`build-std = ["core"]`** — tells Cargo to build the `core` crate from source.
`core` is Rust's dependency-free foundation: it provides primitives like
`Option`, `Result`, iterators, formatting, panic handling, and raw pointer
operations. Everything in `#![no_std]` code depends on `core`.

**`build-std-features = ["compiler-builtins-mem"]`** — enables the pure-Rust
implementations of `memcpy`, `memset`, `memmove`, and `memcmp` inside
`compiler-builtins`. Without this, the linker would look for these C library
functions, which don't exist in a bare-metal environment.

**`link-arg=-Tlinker.ld`** — passes the custom linker script to the linker.
Without this, the linker would place code at a default address that is almost
certainly wrong for bare metal.

**`link-arg=--build-id=none`** — suppresses the linker generating a `.note.gnu.build-id`
section. This section is useful for debugging with a full OS but meaningless
here, and including it would slightly grow the binary and could confuse the
layout.

**`target-cpu=cortex-a7`** — tells LLVM which exact CPU it is targeting. LLVM
uses this to pick the right instruction scheduling, and to enable/disable
specific instruction extensions (like hardware divide, which the Cortex-A7
supports).

### `Cargo.toml` profiles

```toml
[profile.dev]
panic = "abort"

[profile.release]
panic = "abort"
lto = true
opt-level = "s"
```

**`panic = "abort"`** — when a panic occurs, immediately execute an undefined
instruction (or similar hard abort) instead of unwinding the stack. Stack
unwinding requires runtime support that doesn't exist in a bare-metal `#![no_std]`
program. Using `abort` keeps the binary small and avoids needing unwind tables.

**`lto = true`** (link-time optimisation) — allows the compiler to optimise
across crate boundaries. On embedded targets this typically produces significantly
smaller binaries because unused code can be eliminated more aggressively.

**`opt-level = "s"`** — optimise for binary size rather than speed. For a small
microkernel this is almost always the right choice.

### `#![no_std]` and `#![no_main]`

At the top of `src/main.rs`:

```rust
#![no_std]
#![no_main]
```

**`#![no_std]`** — tells Rust not to link against `std` (the standard library).
`std` depends on an operating system for I/O, threads, heap allocation, etc.
Without an OS, `std` is simply unavailable. `#![no_std]` programs can still use
`core`, which contains everything that doesn't need an OS.

**`#![no_main]`** — tells Rust not to generate the normal `main` function entry
point. The normal entry point is a wrapper the C runtime (`crt0`) calls after
setting up argc/argv, which again doesn't exist here. Instead, the entry point
is defined in assembly (`_start`) and calls `kernel_main` directly via `extern "C"`.

---

## 5. The Linker Script — `linker.ld`

```
ENTRY(_start)

SECTIONS
{
    . = 0x8000;

    .text :
    {
        KEEP(*(.text._start))
        *(.text .text.*)
    }

    .rodata ALIGN(4) : { *(.rodata .rodata.*) }
    .data   ALIGN(4) : { *(.data   .data.*  ) }

    . = ALIGN(4);
    __bss_start = .;
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) }
    . = ALIGN(4);
    __bss_end = .;

    /DISCARD/ : { *(.comment) *(.ARM.exidx*) *(.ARM.extab*) *(.note*) }
}
```

The linker script controls exactly how the compiled object files are
assembled into a final binary.

**`. = 0x8000`** — the "location counter". Every address in the binary will be
calculated relative to this base. This must match where the RPi firmware loads
the kernel.

**`KEEP(*(.text._start))`** — forces the `_start` function to be placed first in
the `.text` section, at exactly `0x8000`. Without `KEEP`, the linker might
reorder sections and `_start` wouldn't be the first instruction. The firmware
jumps blindly to `0x8000`, so the first instruction must be `_start`.

**`ALIGN(4)`** — aligns the current address to a 4-byte boundary. ARM loads and
stores of 32-bit values must be 4-byte aligned; unaligned access causes a fault
on ARMv7-A in the default configuration.

**`__bss_start` and `__bss_end`** — these symbols are exported to the linker
map and are readable from Rust/assembly code. The boot assembly uses them to
zero the BSS section. This is essential — Rust's semantics guarantee that static
variables initialised to zero actually start as zero, but the firmware just
dumps the binary into RAM without initialising it.

**`.bss (NOLOAD)`** — the BSS section holds uninitialised (zero-initialised)
static variables. `(NOLOAD)` means it is not included in the binary file on
disk; the boot code initialises it at runtime. This keeps `kernel7.img` small.

**`/DISCARD/`** — throw away sections the firmware doesn't need:
- `.comment` — compiler version string embedded by GCC/Clang
- `.ARM.exidx` / `.ARM.extab` — ARM exception unwinding tables. These are used
  for C++ exceptions and Rust panic unwinding, neither of which works here since
  we use `panic = "abort"`. Keeping them would just waste space.
- `.note` — GNU build notes, including the build-id that `--build-id=none` also
  suppresses.

---

## 6. Boot Assembly — `src/boot.rs`

This file contains ARM32 assembly that runs before any Rust code. It must be
hand-written because Rust requires a valid stack and zeroed BSS before it can
run safely.

```rust
use core::arch::global_asm;

global_asm!(r#"
.arm
.section .text._start
.global _start
_start:
    ...
"#);
```

`global_asm!` embeds raw assembly directly into the compiled object file.
`.arm` forces 32-bit ARM encoding (as opposed to 16-bit Thumb2).
`.section .text._start` is the section name matched by `KEEP(*(.text._start))`
in the linker script — this is how `_start` ends up first.

### Parking Secondary Cores

```asm
mrc     p15, 0, r1, c0, c0, 5    @ read MPIDR into r1
ands    r1, r1, #0x3              @ keep bits [1:0] = CPU ID
bne     .Lpark                    @ non-zero = not core 0, park it
```

The BCM2836 has four Cortex-A7 cores. All four start executing simultaneously
when the firmware releases them. If all four ran `kernel_main`, they would step
on each other's memory, corrupt the stack, and crash immediately.

**MPIDR** (Multiprocessor Affinity Register) is a coprocessor register that
holds the core's ID in bits `[1:0]`. Reading it with `mrc p15, 0, r1, c0, c0, 5`
gives `r1 = 0` on core 0, `r1 = 1` on core 1, etc. The `ands` instruction
masks off everything above bit 1 and sets the Zero flag if the result is 0.
`bne .Lpark` jumps to the parking loop for any non-zero core ID.

The parking loop:
```asm
.Lpark:
    wfe             @ Wait For Event — halts the core, very low power
    b       .Lpark  @ loop back in case of spurious wake
```

`wfe` puts the core in a low-power standby. It wakes on an `sev` (Send Event)
instruction from another core, or on certain interrupt conditions. Since we never
send an event and interrupts are disabled, these cores stay parked for the entire
lifetime of the kernel. To later bring up secondary cores (SMP), you would write
a jump address to the spin-table and send `sev`.

### Dropping from HYP to SVC Mode

```asm
mrs     r0, cpsr
and     r0, r0, #0x1f
cmp     r0, #0x1a       @ 0x1a = HYP mode
bne     .Lsvc_mode
```

ARM Cortex-A7 has multiple exception levels / processor modes. The relevant ones
here are:

| Mode | CPSR[4:0] | Description |
|------|-----------|-------------|
| SVC  | `0x13`    | Supervisor mode — normal privileged operating mode |
| HYP  | `0x1a`    | Hypervisor mode — for running virtualisation |

Depending on the version of `start.elf`, the firmware may leave the CPU in HYP
mode. HYP mode has different register banking and certain instructions (like
`msr cpsr_c`) don't work the same way as in SVC mode. You cannot simply switch
to SVC with `cps` from HYP mode; you must use `eret`.

```asm
mrs     r0, cpsr
bic     r0, r0, #0x1f       @ clear mode bits
orr     r0, r0, #0x13       @ set SVC mode
orr     r0, r0, #0xc0       @ disable IRQ (bit 7) + FIQ (bit 6)
msr     spsr_hyp, r0        @ write desired CPSR into saved PSR for HYP
adr     r0, .Lsvc_mode      @ PC-relative address of .Lsvc_mode label
msr     elr_hyp, r0         @ write return address
eret                        @ exception return: loads ELR_hyp into PC, SPSR_hyp into CPSR
```

`eret` in HYP mode loads `ELR_hyp` into PC and `SPSR_hyp` into CPSR in one
atomic operation. By pre-loading these with the SVC mode flags and the target
address, `eret` effectively performs a jump-to-SVC-mode, which is the only safe
way to leave HYP mode.

If the firmware already started us in SVC mode, the `cmp r0, #0x1a` comparison
fails and we skip straight to `.Lsvc_mode`.

### Setting Up the Stack

```asm
.Lsvc_mode:
    cpsid   aif         @ disable IRQ, FIQ, and async aborts
    ldr     sp, =_start @ set stack pointer to 0x8000
```

**`cpsid aif`** disables all three interrupt sources (IRQ, FIQ, async abort).
With no interrupt vector table and no handlers, any interrupt would jump to
address 0, which contains garbage, causing a crash.

**Stack at `_start` (0x8000)** — The stack grows downward (ARM convention). By
placing the stack pointer at `0x8000`, the stack grows into the region
`0x0000`–`0x7FFF`. Since nothing important is in that range (the firmware data
there is no longer needed), there is roughly 32 KiB of stack space before the
stack would collide with the ATAGS/DTB at the very beginning of RAM. For this
kernel, that is ample.

### Zeroing BSS

```asm
ldr     r0, =__bss_start
ldr     r1, =__bss_end
mov     r2, #0
.Lbss_loop:
    cmp     r0, r1
    bge     .Lbss_done
    str     r2, [r0], #4    @ store zero, post-increment r0 by 4
    b       .Lbss_loop
.Lbss_done:
    bl      kernel_main
```

The `__bss_start` and `__bss_end` symbols are defined in the linker script.
At link time, they resolve to the start and end addresses of the BSS section.
The loop writes zero to every 4-byte word between these addresses.

Why is this necessary? When `cargo objcopy` creates `kernel7.img`, it only
writes `.text`, `.rodata`, and `.data` to the file. BSS is not written (that's
what `(NOLOAD)` means). RAM at boot contains whatever was in it before — which
on a cold boot is random/uninitialised. Rust's language guarantee is that static
variables start at their declared value (zero for `static mut X: u32 = 0`), so
the boot code must fulfil that guarantee manually.

**`bl kernel_main`** calls the Rust entry point. `bl` (Branch with Link) saves
the return address in `lr`, but since `kernel_main` is declared `-> !` (diverging,
never returns), the code after `.Lbss_done` is the park loop, reached only if
something goes catastrophically wrong.

---

## 7. UART Driver — `src/uart.rs`

UART (Universal Asynchronous Receiver/Transmitter) is a simple serial
communication protocol. Two wires: TX (transmit) and RX (receive). A USB
serial adapter connected to GPIO 14/15 lets you see output and type commands
from a laptop terminal.

The BCM2836 has two UARTs:
- **PL011** (UART0) — a full-featured UART, requires the GPU to release it
- **Mini UART** (UART1, AUX) — simpler, no GPU contention, easier to set up

This project uses the Mini UART.

### MMIO Base Addresses

```rust
const MMIO_BASE: usize = 0x3F00_0000;
const GPIO_BASE: usize = MMIO_BASE + 0x0020_0000;
const AUX_BASE:  usize = MMIO_BASE + 0x0021_5000;
```

**Memory-mapped I/O (MMIO)** means peripheral registers are mapped into the
CPU's physical address space. Reading or writing to these addresses causes
hardware side-effects rather than RAM access. The BCM2836 maps all peripheral
registers starting at `0x3F000000`.

The CPU doesn't "know" these are special — it issues a normal memory load/store.
The SoC's bus fabric routes addresses in the `0x3Fxxxxxx` range to peripheral
hardware instead of DRAM.

### GPIO Alternate Functions

GPIO pins are multiplexed — each pin can serve multiple functions. GPIO 14 and
15 can be:
- Digital I/O (GPIO mode, the default)
- UART0 TX/RX (ALT0)
- Mini UART TX/RX (ALT5)

```rust
let mut sel = read_volatile(GPFSEL1);
sel &= !((0b111 << 12) | (0b111 << 15));  // clear bits for pins 14 & 15
sel |=   (0b010 << 12) | (0b010 << 15);   // set ALT5 (0b010)
write_volatile(GPFSEL1, sel);
```

**GPFSEL1** (GPIO Function Select 1) controls pins 10–19. Each pin uses 3 bits.
Pin 14 is at bits `[14:12]`, pin 15 at bits `[17:15]`. ALT5 = `0b010`.

Why read-modify-write? Other bits in GPFSEL1 control other GPIO pins. Writing
only the target bits preserves the other pins' configuration.

### Pull-up/down Configuration

```rust
write_volatile(GPPUD, 0);               // disable pull resistors
delay(150);
write_volatile(GPPUDCLK0, (1 << 14) | (1 << 15));  // assert clock to pins 14 & 15
delay(150);
write_volatile(GPPUD, 0);
write_volatile(GPPUDCLK0, 0);          // clear the clock
```

The BCM2835/2836 uses an unusual protocol to configure internal pull
resistors. Unlike most microcontrollers where you write directly to a
register, the BCM requires a 4-step dance:

1. Write the desired pull state to GPPUD (0 = none, 1 = pull-down, 2 = pull-up)
2. Wait 150 clock cycles for it to stabilise
3. Write a 1 to GPPUDCLK0 for each pin you want to apply it to
4. Wait 150 cycles, then clear both registers

For UART pins, no pull is needed because the serial adapter holds the lines.

### Baud Rate Divisor

```rust
write_volatile(AUX_MU_BAUD, 270);  // 115200 baud at 250 MHz
```

The Mini UART baud rate formula is:

```
baud_rate = core_clock / (8 * (divisor + 1))
```

Solving for divisor:
```
divisor = core_clock / (8 * baud_rate) - 1
        = 250,000,000 / (8 * 115200) - 1
        = 270.9...
        ≈ 270
```

The `core_freq=250` in `config.txt` locks the core clock to exactly 250 MHz.
Without this, the GPU may set the core clock to various speeds depending on
load/temperature, making the divisor wrong and corrupting serial output.

### Polling TX/RX

```rust
pub fn putc(c: u8) {
    unsafe {
        while read_volatile(AUX_MU_LSR) & 0x20 == 0 {}  // wait for TX FIFO space
        write_volatile(AUX_MU_IO, c as u32);
    }
}

pub fn getc() -> u8 {
    unsafe {
        while read_volatile(AUX_MU_LSR) & 0x01 == 0 {}  // wait for RX data
        (read_volatile(AUX_MU_IO) & 0xFF) as u8
    }
}
```

**LSR** (Line Status Register):
- Bit 0 (`0x01`) — data available in RX FIFO
- Bit 5 (`0x20`) — TX FIFO has space for more data

`read_volatile` is critical here. Without it, the compiler might optimise the
loop body away (it sees an infinite loop that reads the same address each time
and produces no visible effect) or cache the first read result. `read_volatile`
tells the compiler "this read has side-effects; never cache it, never elide it."

### `\n` → `\r\n` Translation

```rust
pub fn puts(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            putc(b'\r');
        }
        putc(b);
    }
}
```

Serial terminals expect `\r\n` (carriage return + line feed) to move to the
next line. Rust string literals use `\n` only. `puts` inserts a `\r` before
every `\n` so terminal emulators display output correctly.

---

## 8. VideoCore Mailbox — `src/mailbox.rs`

The VideoCore GPU controls several hardware resources that the ARM CPU cannot
access directly, including the HDMI framebuffer. Communication happens through
a **mailbox** — a small hardware message queue between the ARM and the GPU.

### Physical Addresses

```rust
const MBOX_BASE: usize = MMIO_BASE + 0x0000_B880;

const MBOX_READ:   *mut u32 = (MBOX_BASE + 0x00) as *mut u32;
const MBOX_STATUS: *mut u32 = (MBOX_BASE + 0x18) as *mut u32;
const MBOX_WRITE:  *mut u32 = (MBOX_BASE + 0x20) as *mut u32;
```

There are actually several mailbox channels (0–15). This code uses channel 8,
the **property channel**, which is the standard way to request GPU services
(framebuffer, power, clocks, etc.).

### The Message Buffer

```rust
#[repr(C, align(16))]
pub struct Msg(pub [u32; 36]);
```

**`#[repr(C)]`** — use C layout rules. Rust is normally free to reorder or
pad struct fields however it wants. `repr(C)` guarantees the layout matches
what C (and thus the GPU firmware, which was written in C) expects.

**`align(16)`** — the buffer must be 16-byte aligned. The lower 4 bits of the
address passed to the mailbox are used to encode the channel number. If the
buffer isn't aligned to 16 bytes, those bits would overlap with the actual
address bits and the GPU would receive a wrong address or wrong channel.

### Message Format

A property channel message is a flat array of `u32` words with this layout:

```
[0]     total buffer size in bytes (must include this word)
[1]     request/response code (0 = request, GPU writes 0x80000000 = success)
[2..n]  property tags
[n+1]   0x00000000 end tag
```

Each **property tag** has the format:
```
[+0]    tag identifier (e.g., 0x00048003 = set physical width/height)
[+1]    value buffer size in bytes
[+2]    request/response indicator (0 for request; GPU sets bit 31 = 1 for response)
[+3..n] value data
```

The framebuffer init message in `fb.rs` packs multiple tags into one message
because the GPU processes them atomically — you can set the size, depth, pixel
order, and allocate the buffer all in a single round-trip.

### Sending a Message

```rust
pub fn call(msg: &mut Msg) -> bool {
    let addr = msg as *mut Msg as u32;

    unsafe {
        core::arch::asm!("dsb sy", options(nostack));

        while read_volatile(MBOX_STATUS) & MBOX_FULL != 0 {}
        write_volatile(MBOX_WRITE, (addr & !0xF) | CHAN_PROP);

        loop {
            while read_volatile(MBOX_STATUS) & MBOX_EMPTY != 0 {}
            let r = read_volatile(MBOX_READ);
            if r & 0xF == CHAN_PROP && r & !0xF == addr & !0xF {
                return msg.0[1] == 0x8000_0000;
            }
        }
    }
}
```

**`dsb sy`** — Data Synchronisation Barrier. ARM CPUs have write buffers; a
store to memory might not have actually reached RAM by the time the next
instruction executes. The GPU reads the buffer from RAM via the bus. Without the
barrier, the GPU might see a partially-written message. `dsb sy` ensures all
previous stores have reached their destination before the CPU proceeds.

**`(addr & !0xF) | CHAN_PROP`** — the write combines the message address with
the channel number in the lower 4 bits. `addr & !0xF` zeroes the lower nibble
(guaranteed zero anyway because of the 16-byte alignment), then `| 8` sets the
channel.

**Response polling** — the GPU may respond on any channel, not necessarily the
one you sent to. The read loop checks both that the channel matches (`r & 0xF == CHAN_PROP`)
and the address matches (`r & !0xF == addr & !0xF`). This handles concurrent
mailbox traffic (e.g., the GPU notifying about power events).

**Success check** — `msg.0[1] == 0x8000_0000`. The GPU writes `0x80000000` to
word 1 to indicate success. Any other value means the request failed.

---

## 9. Framebuffer Driver — `src/fb.rs`

The framebuffer is a region of RAM that the GPU continuously reads and converts
to HDMI signal. Each pixel is 4 bytes (32bpp), laid out as `0x00RRGGBB`.
Writing to this memory causes pixels to appear on screen.

### State

```rust
static mut READY:  bool     = false;
static mut BASE:   *mut u32 = core::ptr::null_mut();
static mut WIDTH:  u32      = 0;
static mut HEIGHT: u32      = 0;
static mut PITCH:  u32      = 0;  // bytes per row
static mut CX:     u32      = 0;  // cursor x in pixels
static mut CY:     u32      = 0;  // cursor y in pixels
```

`static mut` in Rust is inherently unsafe to access (multiple threads could race
on it). Since this is a single-core kernel with no preemption, all accesses are
trivially safe — but Rust still requires `unsafe` blocks for any access.

**PITCH** is not the same as `WIDTH * 4`. The GPU may add padding bytes at the
end of each row to align rows to a power-of-two boundary (e.g., rows might be
padded to 4096 bytes even if they're only 4096 bytes wide). Always use PITCH
(not WIDTH) to calculate the offset of the next row.

### Initialisation

The mailbox message for the framebuffer uses 7 property tags:

| Tag ID       | Purpose                    | In       | Out                   |
|--------------|----------------------------|----------|-----------------------|
| `0x00048003` | Set physical width/height  | 1024×768 | (confirmed)           |
| `0x00048004` | Set virtual width/height   | 1024×768 | (confirmed)           |
| `0x00048009` | Set virtual offset         | 0,0      | (confirmed)           |
| `0x00048005` | Set depth (bits/pixel)     | 32       | (confirmed)           |
| `0x00048006` | Set pixel order            | 1 (RGB)  | (confirmed)           |
| `0x00040001` | Allocate framebuffer       | 4096 align | bus address, size   |
| `0x00040008` | Get pitch                  | —        | bytes per row         |

**Physical vs. virtual dimensions** — physical is what the monitor sees;
virtual is what the CPU sees. Setting them equal gives a simple 1:1 display
with no panning. You could set virtual larger than physical to create an
off-screen buffer area.

**Bus address masking**:
```rust
let base = m.0[28] & 0x3FFF_FFFF;
```

The GPU returns a **bus address**, not an ARM physical address. On the BCM2836
the bus address space has `0xC0000000` as the L2-cache-coherent alias of physical
RAM. Masking with `0x3FFF_FFFF` strips the top two bits and gives the ARM
physical address. Writing to the unmasked bus address from the ARM would access
wrong memory or fail entirely.

### Pixel Addressing

```rust
fn pset(x: u32, y: u32, color: u32) {
    unsafe {
        write_volatile(BASE.add((y * (PITCH / 4) + x) as usize), color);
    }
}
```

`BASE` is a `*mut u32`, so `.add(n)` advances by `n * 4` bytes (one `u32`).
The formula is `y * (PITCH / 4) + x` because:
- PITCH is in bytes; dividing by 4 gives the number of u32 words per row
- `y * (PITCH / 4)` is the row offset in words
- `+ x` is the column offset in words

`write_volatile` is needed for the same reason as with UART — the GPU reads
this memory asynchronously, so the compiler must not optimise away or reorder
these writes.

### Drawing a Character

```rust
fn draw_char(c: u8, px: u32, py: u32) {
    let glyph = &font::FONT[if (0x20..=0x7E).contains(&c) { (c - 0x20) as usize } else { 0 }];
    for row in 0..CH {
        let bits = glyph[row as usize];
        for col in 0..CW {
            pset(px + col, py + row, if bits & (0x80 >> col) != 0 { FG } else { BG });
        }
    }
}
```

The font is an 8×8 bitmap. Each glyph is 8 bytes; each byte represents one
row of pixels; bit 7 (MSB) is the leftmost pixel.

`0x80 >> col` creates a bitmask: `0x80` for col=0, `0x40` for col=1, etc.
If that bit is set in the glyph byte, draw the foreground colour; otherwise draw
the background. This renders the entire character cell including background,
which is how backspace (erasing a character) is implemented cleanly.

### Scrolling

```rust
fn scroll() {
    unsafe {
        let row_bytes = (CH * PITCH) as usize;     // bytes in one character row
        let total     = (HEIGHT * PITCH) as usize;  // bytes in entire framebuffer
        let fb = BASE as *mut u8;
        core::ptr::copy(fb.add(row_bytes), fb, total - row_bytes);  // shift up
        core::ptr::write_bytes(fb.add(total - row_bytes), 0, row_bytes); // clear last row
        CY -= CH;
    }
}
```

`core::ptr::copy` is `memmove` — it handles overlapping source and destination
correctly. Here it shifts the framebuffer content up by one character row (CH=8
pixels × PITCH bytes). Then the last row is zeroed.

Note this operates on the raw pixel buffer (cast to `*mut u8`), not on text.
There is no text buffer — the framebuffer *is* the display state. This means
you cannot "reflow" text if you change font size, but it keeps the code simple.

---

## 10. Bitmap Font — `src/font.rs`

```rust
pub const W: usize = 8;
pub const H: usize = 8;

pub static FONT: [[u8; H]; 95] = [ ... ];
```

The font covers exactly the 95 printable ASCII characters from space (0x20)
to tilde (0x7E). The array index is `c - 0x20`, so `FONT[0]` = space,
`FONT[1]` = `!`, `FONT[33]` = `A`, etc.

Each glyph is 8 bytes. Each byte is one row of pixels from top to bottom.
Within each byte, bit 7 (MSB, `0x80`) is the leftmost pixel, bit 0 is the
rightmost.

Example — the letter `A` (0x41, index 33):
```
[0x0C, 0x1E, 0x33, 0x33, 0x3F, 0x33, 0x33, 0x00]

Row 0: 0x0C = 0000 1100  →  . . . . X X . .
Row 1: 0x1E = 0001 1110  →  . . . X X X X .
Row 2: 0x33 = 0011 0011  →  . . X X . . X X
Row 3: 0x33 = 0011 0011  →  . . X X . . X X
Row 4: 0x3F = 0011 1111  →  . . X X X X X X  (the crossbar)
Row 5: 0x33 = 0011 0011  →  . . X X . . X X
Row 6: 0x33 = 0011 0011  →  . . X X . . X X
Row 7: 0x00 = 0000 0000  →  . . . . . . . .  (descender space)
```

The font is stored as `static` (in the `.rodata` section) rather than generated
at runtime. It costs about 95 × 8 = 760 bytes in the binary, which is negligible.

To use a different font: replace the byte arrays in `FONT` with your own
glyphs. To use a different size: change `W` and `H` and update the array
dimensions. The rest of `fb.rs` uses `CW`/`CH` which derive from `font::W`/`font::H`.

---

## 11. Console Abstraction — `src/console.rs`

```rust
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
```

This is a thin multiplexer. Every character goes to both UART and framebuffer
simultaneously. If the framebuffer isn't ready (`READY = false`), `fb::putc`
returns immediately, so UART-only operation is automatic fallback.

**`fmt::Write`** is Rust's trait for write-formatting targets. Implementing it
enables `write!` and `writeln!` macros:

```rust
let _ = write!(console::Writer, "Value: {}\r\n", some_number);
```

`console::Writer` is a zero-sized struct. It holds no state; all state lives in
the `uart` and `fb` modules as statics. Creating a `Writer` costs nothing.

The `let _ =` discards the `Result` from `write!`. In a no-std environment
without panic recovery, there's nothing useful to do with a formatting error,
and the `?` operator is unavailable at the top level.

---

## 12. The Kernel Shell — `src/main.rs`

### Entry Point

```rust
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
```

**`#[no_mangle]`** — by default Rust mangles function names (appending a hash)
to support overloading and separate compilation. `no_mangle` tells Rust to
export this function with its exact name `kernel_main`, which is what the
assembly `bl kernel_main` instruction references.

**`extern "C"`** — use the C calling convention. The ARM C ABI defines how
arguments are passed in registers (r0–r3), how the return address is passed
(in `lr`), and how the stack is maintained. The assembly's `bl` instruction
follows C calling conventions, so the Rust function must match.

**`-> !`** — the diverging return type. This function must never return. If it
did, execution would fall into the post-`bl` park loop in boot assembly. Rust
verifies that every code path in the function either diverges (infinite loop,
panic, etc.) or never reaches the end.

### Panic Handler

```rust
#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    let _ = write!(console::Writer, "\r\n[PANIC] {}\r\n", info);
    loop {}
}
```

In `#![no_std]` code, you must provide a panic handler. Rust calls this when
a `panic!` macro fires, an array is indexed out of bounds, integer overflow
occurs (in debug builds), etc. This implementation prints the panic message to
UART and HDMI, then halts. A real OS would try to recover or reboot.

### Input Loop

```rust
let mut buf = [0u8; 128];
let mut len = 0usize;

loop {
    let c = uart::getc();
    match c {
        b'\r' | b'\n' => { ... run_command ... }
        0x08 | 0x7F   => { ... backspace ... }
        0x20..=0x7E   => { ... printable character ... }
        _ => {}
    }
}
```

The shell is synchronous and single-threaded. `uart::getc()` blocks until a
byte arrives. Characters are accumulated in `buf` until Enter is pressed.

`0x08` is ASCII Backspace; `0x7F` is DEL. Both are sent by common terminal
emulators when the Backspace key is pressed. Both are handled identically.

The `128`-byte buffer means commands longer than 128 characters are silently
truncated. This is fine for a learning kernel; a real shell would use a growable
buffer.

### Command Dispatch

```rust
fn run_command(input: &[u8]) {
    let (verb, args) = match input.iter().position(|&b| b == b' ') {
        Some(i) => (&input[..i], &input[i + 1..]),
        None    => (input, &b""[..]),
    };

    match verb {
        b"help" => { ... }
        b"echo" => { ... }
        b"clear" => { ... }
        b"info" => { ... }
        b"hex" => { ... }
        _ => { ... }
    }
}
```

The input is a raw `&[u8]` (byte slice), not a `&str`. UTF-8 strings aren't used
because the shell only deals with plain ASCII, and byte comparisons (`b"help"`)
are simpler and avoid any UTF-8 validation overhead.

The `hex` command reads a 32-bit hardware register at an arbitrary address:

```rust
let val = unsafe { core::ptr::read_volatile(addr as *const u32) };
```

This is genuinely useful for exploring hardware. For example, you can read the
UART's LSR register at `0x3F215054` or the mailbox status at `0x3F00B898`.

### Hex Parser

```rust
fn parse_hex(s: &[u8]) -> Option<usize> {
    let s = if s.starts_with(b"0x") || s.starts_with(b"0X") { &s[2..] } else { s };
    ...
    val = val.checked_shl(4)?.checked_add(nibble)?;
    ...
}
```

`checked_shl` and `checked_add` return `None` on overflow. The `?` operator
propagates `None` upward, so the function returns `None` for any address too
large to fit in `usize`. On a 32-bit target, `usize` is 32 bits, so any valid
32-bit address is representable.

---

## 13. The SD Card — `config.txt`

```
kernel=kernel7.img
core_freq=250
```

**`kernel=kernel7.img`** — tells `start.elf` to load this filename as the ARM
kernel. The `7` suffix tells the firmware this is a 32-bit ARMv7 binary. (For
a 64-bit ARMv8 kernel the convention is `kernel8.img`; for a 32-bit RPi1 target
it's `kernel.img`.)

**`core_freq=250`** — locks the VPU/core clock to 250 MHz. The Mini UART baud
rate divisor is calculated assuming this clock. Without this setting, the clock
may dynamically change (especially on cold boot vs. warm boot) and the UART baud
rate will be wrong, producing garbage on the serial terminal.

Other useful `config.txt` options for development:
- `enable_uart=1` — ensures UART is enabled even on RPi3+ where it's disabled by default
- `hdmi_force_hotplug=1` — enables HDMI even if no monitor is detected at boot
- `hdmi_group=2` / `hdmi_mode=16` — force 1024×768@60Hz if auto-detection picks wrong mode
- `gpu_mem=16` — reduce GPU memory reservation to the minimum (saves RAM for your kernel)

---

## 14. Building and Flashing

### Build

```powershell
cargo build --release
```

Cargo compiles all crates, links them with `linker.ld`, and produces an ELF
file at `target/armv7a-none-eabi/release/rpi2-os`.

### Convert to Raw Binary

```powershell
cargo objcopy --release -- -O binary kernel7.img
```

The RPi firmware expects a raw binary, not an ELF file. ELF files contain
headers, symbol tables, and section metadata that the firmware doesn't
understand. `objcopy` strips all of that and writes only the raw bytes, starting
from the `.text` section at address `0x8000`.

**Verify** the output is sane:
```powershell
# Check the first 4 bytes — they should be the first ARM instruction
# of _start, which is "mrc p15, 0, r1, c0, c0, 5" = 0xEE101F95
(Get-Content kernel7.img -Raw -Encoding Byte)[0..3]
```

Or with `cargo objdump`:
```powershell
cargo objdump --release -- --disassemble | Select-String -Pattern "_start" -Context 0,10
```

### Flash to SD Card

1. Format the SD card as FAT32 (MBR partition table).
2. Copy `bootcode.bin`, `start.elf`, `fixup.dat` from the
   [raspberrypi/firmware](https://github.com/raspberrypi/firmware) GitHub
   repository (`boot/` folder).
3. Copy `config.txt` and `kernel7.img` from this project.
4. Eject and insert into the RPi2.

---

## 15. Memory Map

```
Physical Address    Contents
─────────────────────────────────────────────────
0x00000000          Firmware data (ATAGS / device tree)
0x00008000          _start (first instruction of kernel)
0x00008000 ↑        .text (code)
             ↑      .rodata (font data, string literals)
             ↑      .data (mutable statics with non-zero init)
             ↑      .bss (zero-initialised statics)
             ↑      (end of kernel image, ~9 KiB)
0x00008000 ↓        Stack (grows downward from 0x8000 toward 0x0000)
             ↓      ~32 KiB of stack space available
─────────────────────────────────────────────────
0x3F000000          MMIO peripherals base
0x3F200000          GPIO registers
0x3F215000          AUX / Mini UART registers
0x3F00B880          VideoCore mailbox
─────────────────────────────────────────────────
0x???????           Framebuffer (GPU-allocated, address returned by mailbox)
                    Typically somewhere above 0x10000000, varies per boot
```

The kernel binary is small (~9 KiB). The vast majority of the 1 GB physical RAM
is unused. Usable addresses are roughly `0x00010000` to `0x3EFFFFFF` (the GPU
reserves the top portion for its own use).

---

## 16. MMIO Register Reference

### GPIO (`0x3F200000`)

| Offset | Name      | Description                              |
|--------|-----------|------------------------------------------|
| `+0x00` | GPFSEL0  | Function select pins 0–9                 |
| `+0x04` | GPFSEL1  | Function select pins 10–19 (14,15 here)  |
| `+0x94` | GPPUD    | Pull-up/down enable                      |
| `+0x98` | GPPUDCLK0 | Pull-up/down clock pins 0–31            |

### AUX/Mini UART (`0x3F215000`)

| Offset | Name          | Description                               |
|--------|---------------|-------------------------------------------|
| `+0x04` | AUX_ENABLES  | Bit 0: enable Mini UART                   |
| `+0x40` | AUX_MU_IO    | Data register (read = RX, write = TX)     |
| `+0x44` | AUX_MU_IER   | Interrupt enable                          |
| `+0x48` | AUX_MU_IIR   | Interrupt identify / FIFO clear           |
| `+0x4C` | AUX_MU_LCR   | Line control (bit 1 = 8-bit mode)         |
| `+0x50` | AUX_MU_MCR   | Modem control                             |
| `+0x54` | AUX_MU_LSR   | Line status (bit 0 = RX ready, bit 5 = TX ready) |
| `+0x60` | AUX_MU_CNTL  | Control (bit 0 = RX enable, bit 1 = TX enable) |
| `+0x68` | AUX_MU_BAUD  | Baud rate divisor                         |

### Mailbox (`0x3F00B880`)

| Offset | Name         | Description                               |
|--------|--------------|-------------------------------------------|
| `+0x00` | MBOX_READ   | Read a message (lower 4 bits = channel)   |
| `+0x18` | MBOX_STATUS | Bit 31 = full (can't write), Bit 30 = empty (nothing to read) |
| `+0x20` | MBOX_WRITE  | Write a message (lower 4 bits = channel)  |

---

## 17. How to Extend This

Here are the natural next steps, in rough order of difficulty.

### Add more shell commands

Add a new `b"mycommand"` arm to the `match verb` in `run_command`. Access
hardware by reading MMIO registers directly with `read_volatile`.

### Read from the mailbox (get board info, clock rates)

Use the same mailbox infrastructure to query things like the board revision or
ARM memory layout. Tag `0x00010001` returns the board model; `0x00030002`
returns the clock rate for a given clock ID.

### Enable interrupts

Set up an ARM exception vector table at address `0x00000000` (or `0xFFFF0000`
with HIVECS). Implement an IRQ handler. The BCM2836 interrupt controller is at
`0x3F00B200`.

For a Mini UART receive interrupt: set `AUX_MU_IER` bit 0, set the IRQ enable
in the interrupt controller, then `cpsie i` to unmask IRQ.

### Add a heap allocator

Implement `GlobalAlloc` against a static byte array. This lets you use `Box`,
`Vec`, and `String` from `alloc`. The simplest bump allocator is ~20 lines. Once
you have a heap, the shell can handle variable-length inputs.

### Read the SD card

The BCM2836 has an EMMC controller at `0x3F300000`. Implementing SD card reads
lets you load files at runtime — executable code, config, or data. This is
non-trivial but well-documented.

### USB keyboard input

Remove the dependency on a serial adapter. The RPi2 has USB ports; there is
an open-source bare-metal USB stack (CSUD) targeting the Synopsys DWC USB OTG
controller at `0x3F980000`.

### Run on all 4 cores (SMP)

The BCM2836 spin-table is at `0x000000D8`–`0x000000E8`. Write a jump address
to the table entry for a core, then issue `sev`. The secondary core will jump
there. You'll need a separate stack for each core and some form of
synchronisation (spinlocks, atomic operations via LDREX/STREX).

### Use the hardware timer

The ARM Generic Timer (available on Cortex-A7) provides a high-resolution
time source via `cntpct_el0` / `cntvct_el0` coprocessor registers. The BCM
system timer is also at `0x3F003000`. Either can be used for delays without
burning cycles in a nop loop.

### Port to Raspberry Pi 4

The BCM2711 (RPi4) uses a different MMIO base (`0xFE000000`), a different UART
(PL011 on GPIO 14/15 in ALT0 rather than Mini UART), and boots in AArch64 mode
by default. The mailbox protocol is the same. The linker base address changes to
`0x80000` (not `0x8000`). The target triple becomes `aarch64-unknown-none`.
