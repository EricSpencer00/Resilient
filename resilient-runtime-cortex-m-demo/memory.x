/* RES-101: placeholder Cortex-M4 memory map.
 *
 * These values are *representative* — 256 KiB FLASH, 64 KiB RAM —
 * chosen because they fit a common STM32F4 / nRF52 class part and
 * keep the `cortex-m-rt` linker happy for a clean build. Real
 * deployments pull origin/length from their chip's datasheet;
 * override this file on a fork if you need exact numbers.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
