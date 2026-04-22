/* ZSA Voyager memory layout
 *
 * 8 KB bootloader at 0x08000000; firmware links at 0x08002000.
 * GD32F303 has 128K flash, 32K main RAM, 8K CCM at 0x10000000.
 *
 * Last 4 KB of flash reserved for RMK's storage (Vial keymap persistence
 * via sequential-storage). StorageConfig::default() uses the last two
 * 2 KB sectors; we shrink the linker-visible FLASH length accordingly so
 * firmware never grows into the storage region.
 */
MEMORY
{
    FLASH : ORIGIN = 0x08002000, LENGTH = 128K - 8K - 4K
    RAM   : ORIGIN = 0x20000000, LENGTH = 32K
    /* CCMRAM : ORIGIN = 0x10000000, LENGTH = 8K */
}
