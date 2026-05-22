use core::arch::global_asm;

global_asm!(
    r#"
.arm
.section .text._start
.global _start
_start:
    // Park all cores except core 0 (MPIDR bits [1:0] = CPU ID)
    mrc     p15, 0, r1, c0, c0, 5
    ands    r1, r1, #0x3
    bne     .Lpark

    // Some firmware versions start us in HYP mode; drop to SVC if so
    mrs     r0, cpsr
    and     r0, r0, #0x1f
    cmp     r0, #0x1a           @ 0x1a = HYP mode
    bne     .Lsvc_mode
    mrs     r0, cpsr
    bic     r0, r0, #0x1f
    orr     r0, r0, #0x13       @ SVC mode
    orr     r0, r0, #0xc0       @ IRQ + FIQ disabled
    msr     spsr_hyp, r0
    adr     r0, .Lsvc_mode
    msr     elr_hyp, r0
    eret

.Lsvc_mode:
    // Disable IRQ and FIQ
    cpsid   aif

    // Stack grows down from load address
    ldr     sp, =_start

    // Zero BSS
    ldr     r0, =__bss_start
    ldr     r1, =__bss_end
    mov     r2, #0
.Lbss_loop:
    cmp     r0, r1
    bge     .Lbss_done
    str     r2, [r0], #4
    b       .Lbss_loop
.Lbss_done:
    bl      kernel_main

    // kernel_main must never return; if it does, park here
.Lpark:
    wfe
    b       .Lpark
    "#
);
