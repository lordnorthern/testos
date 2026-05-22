// VideoCore mailbox — property channel (8).
// The message buffer must be 16-byte aligned; the lower 4 bits of the
// physical address are used to encode the channel number.

use core::ptr::{read_volatile, write_volatile};

const MMIO_BASE: usize = 0x3F00_0000;
const MBOX_BASE: usize = MMIO_BASE + 0x0000_B880;

const MBOX_READ:   *mut u32 = (MBOX_BASE + 0x00) as *mut u32;
const MBOX_STATUS: *mut u32 = (MBOX_BASE + 0x18) as *mut u32;
const MBOX_WRITE:  *mut u32 = (MBOX_BASE + 0x20) as *mut u32;

const MBOX_FULL:  u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;
const CHAN_PROP:   u32 = 8;

#[repr(C, align(16))]
pub struct Msg(pub [u32; 36]);

impl Msg {
    pub const fn new() -> Self {
        Msg([0; 36])
    }
}

/// Send `msg` on the property channel and wait for the GPU's reply.
/// Returns true if the GPU acknowledged success (response code 0x8000_0000).
pub fn call(msg: &mut Msg) -> bool {
    let addr = msg as *mut Msg as u32;

    unsafe {
        // Ensure all buffer writes are visible to the GPU before we ring the bell
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
