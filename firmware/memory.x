/* Memory layout for RP2350 (Raspberry Pi Pico 2) */

MEMORY {
    /* RP2350 has 520KB SRAM split into banks */
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x7F00
    RAM : ORIGIN = 0x10008000, LENGTH = 512K - 32K
    SCRATCH_X : ORIGIN = 0x20000000, LENGTH = 4K
    SCRATCH_Y : ORIGIN = 0x20001000, LENGTH = 4K
}

/* Stack size for the main thread */
_stack_start = ORIGIN(RAM) + LENGTH(RAM);
_stack_size = 8K;
