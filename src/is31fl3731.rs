//! Per-key RGB via two IS31FL3731 drivers on the shared I2C bus:
//!   0x74 (ADDR = GND): left half, 26 keys
//!   0x77 (ADDR = VCC): right half, 26 keys
//!
//! The 52-entry `LED_TABLE` maps a flat LED index to (chip, r_reg,
//! g_reg, b_reg). `KEY_TO_LED` maps matrix (row, col) -> LED index or
//! `NO_LED` for cells with no physical LED. Both come straight from
//! the Voyager's hardware layout.

use embassy_stm32::i2c::{I2c, Master};
use embassy_stm32::mode::Blocking;
use embassy_time::Timer;

pub const ADDR_LEFT: u8 = 0x74;
pub const ADDR_RIGHT: u8 = 0x77;

const CHIP_COUNT: usize = 2;
const PWM_BYTES: usize = 144;
const LED_CONTROL_COUNT: u8 = 18;
const PWM_CHUNKS: u8 = 9;
const CHUNK_BYTES: usize = 16;

const REG_COMMAND: u8 = 0xFD;
const CMD_FRAME_1: u8 = 0x00;
const CMD_FUNCTION: u8 = 0x0B;

const FN_CONFIG: u8 = 0x00;
const FN_PICTURE_DISPLAY: u8 = 0x01;
const FN_AUDIO_SYNC: u8 = 0x06;
const FN_SHUTDOWN: u8 = 0x0A;
const CONFIG_MODE_PICTURE: u8 = 0x00;

const FRAME_REG_LED_CONTROL: u8 = 0x00;
const FRAME_REG_PWM: u8 = 0x24;

/// Encode the IS31FL3731 "Cn_m" register name: `(n-1)*16 + (m-1)`.
const fn c(n: u8, m: u8) -> u8 {
    (n - 1) * 16 + (m - 1)
}

#[derive(Copy, Clone)]
struct LedEntry {
    chip: u8,
    r: u8,
    g: u8,
    b: u8,
}

const fn e(chip: u8, r: u8, g: u8, b: u8) -> LedEntry {
    LedEntry { chip, r, g, b }
}

/// 52 physical LEDs in the order the Voyager wires them (left half
/// rows 0-5 top-to-bottom, then right half rows 6-11).
#[rustfmt::skip]
const LED_TABLE: [LedEntry; 52] = [
    // Left half (chip 0)
    e(0, c(2,2),  c(1,2),  c(4,3)),
    e(0, c(2,3),  c(1,3),  c(3,3)),
    e(0, c(2,4),  c(1,4),  c(3,4)),
    e(0, c(2,5),  c(1,5),  c(3,5)),
    e(0, c(2,6),  c(1,6),  c(3,6)),
    e(0, c(2,7),  c(1,7),  c(3,7)),
    e(0, c(2,8),  c(1,8),  c(3,8)),
    e(0, c(8,1),  c(7,1),  c(9,1)),
    e(0, c(8,2),  c(7,2),  c(9,2)),
    e(0, c(8,3),  c(7,3),  c(9,3)),
    e(0, c(8,4),  c(7,4),  c(9,4)),
    e(0, c(8,5),  c(7,5),  c(9,5)),
    e(0, c(8,6),  c(7,6),  c(9,6)),
    e(0, c(2,10), c(1,10), c(4,11)),
    e(0, c(2,11), c(1,11), c(3,11)),
    e(0, c(2,12), c(1,12), c(3,12)),
    e(0, c(2,13), c(1,13), c(3,13)),
    e(0, c(2,14), c(1,14), c(3,14)),
    e(0, c(2,15), c(1,15), c(3,15)),
    e(0, c(2,16), c(1,16), c(3,16)),
    e(0, c(8,9),  c(7,9),  c(9,9)),
    e(0, c(8,10), c(7,10), c(9,10)),
    e(0, c(8,11), c(7,11), c(9,11)),
    e(0, c(8,12), c(7,12), c(9,12)),
    e(0, c(8,13), c(7,13), c(9,13)),
    e(0, c(8,14), c(7,14), c(9,14)),
    // Right half (chip 1)
    e(1, c(2,7),  c(1,7),  c(3,7)),
    e(1, c(2,6),  c(1,6),  c(3,6)),
    e(1, c(2,5),  c(1,5),  c(3,5)),
    e(1, c(2,4),  c(1,4),  c(3,4)),
    e(1, c(2,3),  c(1,3),  c(3,3)),
    e(1, c(2,2),  c(1,2),  c(4,3)),
    e(1, c(8,5),  c(7,5),  c(9,5)),
    e(1, c(8,4),  c(7,4),  c(9,4)),
    e(1, c(8,3),  c(7,3),  c(9,3)),
    e(1, c(8,2),  c(7,2),  c(9,2)),
    e(1, c(8,1),  c(7,1),  c(9,1)),
    e(1, c(2,8),  c(1,8),  c(3,8)),
    e(1, c(2,14), c(1,14), c(3,14)),
    e(1, c(2,13), c(1,13), c(3,13)),
    e(1, c(2,12), c(1,12), c(3,12)),
    e(1, c(2,11), c(1,11), c(3,11)),
    e(1, c(2,10), c(1,10), c(4,11)),
    e(1, c(8,6),  c(7,6),  c(9,6)),
    e(1, c(8,12), c(7,12), c(9,12)),
    e(1, c(8,11), c(7,11), c(9,11)),
    e(1, c(8,10), c(7,10), c(9,10)),
    e(1, c(8,9),  c(7,9),  c(9,9)),
    e(1, c(2,16), c(1,16), c(3,16)),
    e(1, c(2,15), c(1,15), c(3,15)),
    e(1, c(8,14), c(7,14), c(9,14)),
    e(1, c(8,13), c(7,13), c(9,13)),
];

const NO_LED: u8 = 0xFF;

/// Physical x position (0..15 across both halves) of each LED in
/// `LED_TABLE` order. Used by the rainbow animation to phase hue by
/// horizontal position.
#[rustfmt::skip]
const LED_X: [u8; 52] = [
    // Left half (chip 0)
    0, 1, 2, 3, 4, 5,       // row 0 alphas
    0, 1, 2, 3, 4, 5,       // row 1
    0, 1, 2, 3, 4, 5,       // row 2
    0, 1, 2, 3, 4,          // row 3 (no col 6)
    5,                      // `B` at [4,4]
    5, 6,                   // left thumbs [5,0] [5,1]
    // Right half (chip 1)
    10, 11, 12, 13, 14, 15, // row 6
    10, 11, 12, 13, 14, 15, // row 7
    10, 11, 12, 13, 14, 15, // row 8
    10,                     // `N` at [10,2]
    11, 12, 13, 14, 15,     // row 9 (no col 0)
    9, 10,                  // right thumbs [11,5] [11,6]
];

/// Adafruit-style hue wheel: `pos` in 0..=255 sweeps red → green →
/// blue → red. Returns full-saturation RGB; scale by `brightness` for
/// overall intensity control.
fn wheel(pos: u8) -> (u8, u8, u8) {
    if pos < 85 {
        (255 - pos * 3, pos * 3, 0)
    } else if pos < 170 {
        let p = pos - 85;
        (0, 255 - p * 3, p * 3)
    } else {
        let p = pos - 170;
        (p * 3, 0, 255 - p * 3)
    }
}

fn scale(c: u8, brightness: u8) -> u8 {
    ((c as u16 * brightness as u16) / 255) as u8
}

/// Matrix [row][col] -> index into `LED_TABLE`, or `NO_LED` for cells
/// without a physical switch.
#[rustfmt::skip]
const KEY_TO_LED: [[u8; 7]; 12] = [
    // Left half (rows 0-5). Col 0 is thumb-only; cols 1-6 are letters.
    [NO_LED,  0,  1,  2,  3,  4,  5],  // row 0
    [NO_LED,  6,  7,  8,  9, 10, 11],  // row 1
    [NO_LED, 12, 13, 14, 15, 16, 17],  // row 2
    [NO_LED, 18, 19, 20, 21, 22, NO_LED], // row 3
    [NO_LED, NO_LED, NO_LED, NO_LED, 23, NO_LED, NO_LED], // row 4 (B)
    [    24,     25, NO_LED, NO_LED, NO_LED, NO_LED, NO_LED], // row 5 (thumbs)
    // Right half (rows 6-11). Col 6 is thumb-only; cols 0-5 are letters.
    [    26, 27, 28, 29, 30, 31, NO_LED], // row 6
    [    32, 33, 34, 35, 36, 37, NO_LED], // row 7
    [    38, 39, 40, 41, 42, 43, NO_LED], // row 8
    [NO_LED, 45, 46, 47, 48, 49, NO_LED], // row 9
    [NO_LED, NO_LED, 44, NO_LED, NO_LED, NO_LED, NO_LED], // row 10 (N)
    [NO_LED, NO_LED, NO_LED, NO_LED, NO_LED,     50,     51], // row 11 (thumbs)
];

fn write_reg(i2c: &mut I2c<'_, Blocking, Master>, addr: u8, reg: u8, val: u8) -> Result<(), ()> {
    i2c.blocking_write(addr, &[reg, val]).map_err(|_| ())
}

fn select_page(i2c: &mut I2c<'_, Blocking, Master>, addr: u8, page: u8) -> Result<(), ()> {
    write_reg(i2c, addr, REG_COMMAND, page)
}

/// Bring a chip into picture mode with every LED control bit enabled
/// and the PWM buffer cleared. After this, the chip is ready to
/// receive per-key PWM writes via `Rgb::flush`.
pub async fn init_chip(i2c: &mut I2c<'_, Blocking, Master>, addr: u8) -> Result<(), ()> {
    select_page(i2c, addr, CMD_FUNCTION)?;
    write_reg(i2c, addr, FN_SHUTDOWN, 0x00)?;
    Timer::after_millis(10).await;

    write_reg(i2c, addr, FN_CONFIG, CONFIG_MODE_PICTURE)?;
    write_reg(i2c, addr, FN_PICTURE_DISPLAY, 0x00)?;
    write_reg(i2c, addr, FN_AUDIO_SYNC, 0x00)?;

    select_page(i2c, addr, CMD_FRAME_1)?;
    for i in 0..LED_CONTROL_COUNT {
        write_reg(i2c, addr, FRAME_REG_LED_CONTROL + i, 0xFF)?;
    }
    let mut buf = [0u8; 1 + CHUNK_BYTES];
    for chunk in 0..PWM_CHUNKS {
        buf[0] = FRAME_REG_PWM + chunk * CHUNK_BYTES as u8;
        i2c.blocking_write(addr, &buf).map_err(|_| ())?;
    }

    select_page(i2c, addr, CMD_FUNCTION)?;
    write_reg(i2c, addr, FN_SHUTDOWN, 0x01)?;
    select_page(i2c, addr, CMD_FRAME_1)?;
    Ok(())
}

/// In-memory PWM state for both chips; flush to push to hardware.
pub struct Rgb {
    bufs: [[u8; PWM_BYTES]; CHIP_COUNT],
    dirty: [bool; CHIP_COUNT],
}

impl Rgb {
    pub const fn new() -> Self {
        Self {
            bufs: [[0; PWM_BYTES]; CHIP_COUNT],
            dirty: [false; CHIP_COUNT],
        }
    }

    pub fn set_key(&mut self, row: usize, col: usize, r: u8, g: u8, b: u8) {
        if row >= KEY_TO_LED.len() || col >= KEY_TO_LED[0].len() {
            return;
        }
        let led = KEY_TO_LED[row][col];
        if led == NO_LED {
            return;
        }
        let entry = LED_TABLE[led as usize];
        let chip = entry.chip as usize;
        self.bufs[chip][entry.r as usize] = r;
        self.bufs[chip][entry.g as usize] = g;
        self.bufs[chip][entry.b as usize] = b;
        self.dirty[chip] = true;
    }

    pub fn set_all(&mut self, r: u8, g: u8, b: u8) {
        for entry in &LED_TABLE {
            let chip = entry.chip as usize;
            self.bufs[chip][entry.r as usize] = r;
            self.bufs[chip][entry.g as usize] = g;
            self.bufs[chip][entry.b as usize] = b;
            self.dirty[chip] = true;
        }
    }

    /// Cycle-left-right style rainbow: each key's hue is a function of
    /// its physical x position (8 units of hue per column) plus a
    /// time-varying phase. Full saturation; `brightness` scales the
    /// whole wheel down to non-blinding intensity.
    pub fn paint_rainbow(&mut self, phase: u8, brightness: u8) {
        for (idx, entry) in LED_TABLE.iter().enumerate() {
            let hue = (LED_X[idx] as u16 * 8 + phase as u16) as u8;
            let (r, g, b) = wheel(hue);
            let chip = entry.chip as usize;
            self.bufs[chip][entry.r as usize] = scale(r, brightness);
            self.bufs[chip][entry.g as usize] = scale(g, brightness);
            self.bufs[chip][entry.b as usize] = scale(b, brightness);
            self.dirty[chip] = true;
        }
    }

    pub fn flush(&mut self, i2c: &mut I2c<'_, Blocking, Master>) -> Result<(), ()> {
        for chip in 0..CHIP_COUNT {
            if !self.dirty[chip] {
                continue;
            }
            let addr = if chip == 0 { ADDR_LEFT } else { ADDR_RIGHT };
            let mut buf = [0u8; 1 + CHUNK_BYTES];
            for chunk in 0..PWM_CHUNKS {
                let base = chunk as usize * CHUNK_BYTES;
                buf[0] = FRAME_REG_PWM + chunk * CHUNK_BYTES as u8;
                buf[1..].copy_from_slice(&self.bufs[chip][base..base + CHUNK_BYTES]);
                i2c.blocking_write(addr, &buf).map_err(|_| ())?;
            }
            self.dirty[chip] = false;
        }
        Ok(())
    }
}
