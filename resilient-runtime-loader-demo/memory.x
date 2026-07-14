/* RES-3987 (D-E1): memory map for QEMU's `lm3s6965evb` machine —
 * `docs/EMBEDDED_PIPELINE.md` section 4 names this as the Cortex-M
 * QEMU CI target (`qemu-system-arm -M lm3s6965evb -cpu cortex-m4
 * -semihosting-config enable=on,target=native -kernel <elf>`).
 * Unlike `resilient-runtime-cortex-m-demo/memory.x` (a generic
 * STM32F4-class placeholder at 0x08000000), this binary is meant to
 * actually run under QEMU, so the memory map must match the
 * emulated machine rather than a real board.
 */

MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
