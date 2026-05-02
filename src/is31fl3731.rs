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
#[cfg(feature = "palettefx")]
use rmk_palettefx::color::{Hsv, hsv_to_rgb};
#[cfg(feature = "palettefx")]
use rmk_palettefx::layout::LedLayout;

use crate::keymap::{COL, ROW};

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

/// Number of physical RGB LEDs across both halves.
pub const LED_COUNT: usize = 52;

/// 52 physical LEDs in the order the Voyager wires them (left half
/// rows 0-5 top-to-bottom, then right half rows 6-11).
#[rustfmt::skip]
const LED_TABLE: [LedEntry; LED_COUNT] = [
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

/// Physical x position (0..15 across both halves) of each LED in
/// `LED_TABLE` order. Used by the rainbow animation (hue phase offset)
/// and, scaled by 17, by the `LedLayout` impl for rmk-palettefx.
#[rustfmt::skip]
#[cfg(not(feature = "palettefx"))]
const LED_X: [u8; LED_COUNT] = [
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

/// Physical (x, y) positions for each LED in `LED_TABLE` order.
///
/// x spans 0–224 in physical coordinates, scaled to 0–255 (×255/224).
/// y uses the *same* scale factor (×255/224 applied to the 0–58 y range),
/// giving y ≈ 6–66. Both axes share one scale factor so the rotation matrix
/// in Flow (and the polar math in Vortex/Ripple) is isotropic.
#[cfg(feature = "palettefx")]
#[rustfmt::skip]
const LED_POS: [(u8, u8); LED_COUNT] = [
    // Left half (chip 0)
    (  0, 11), ( 19, 11), ( 39,  9), ( 59,  6), ( 79,  9), ( 98, 11), // row 0 (numbers)
    (  0, 24), ( 19, 24), ( 39, 22), ( 59, 19), ( 79, 22), ( 98, 24), // row 1
    (  0, 37), ( 19, 37), ( 39, 34), ( 59, 32), ( 79, 34), ( 98, 37), // row 2 (home)
    (  0, 49), ( 19, 49), ( 39, 47), ( 59, 44), ( 79, 47),            // row 3 (cols 1-5)
    ( 98, 49),                                                          // row 4 (B drop)
    ( 98, 60), (109, 66),                                              // row 5 (thumbs)
    // Right half (chip 1)
    (157, 11), (176, 11), (196,  9), (216,  6), (236,  9), (255, 11), // row 6 (numbers)
    (157, 24), (176, 24), (196, 22), (216, 19), (236, 22), (255, 24), // row 7
    (157, 37), (176, 37), (196, 34), (216, 32), (236, 34), (255, 37), // row 8 (home)
    (157, 49),                                                          // row 10 (N drop)
    (176, 49), (196, 47), (216, 44), (236, 47), (255, 49),            // row 9 (cols 1-5)
    (146, 66), (157, 60),                                              // row 11 (thumbs)
];

/// LED position table for rmk-palettefx effects. Returns (x, y) from
/// `LED_POS` - physical coordinates scaled to 0..=255.
#[cfg(feature = "palettefx")]
pub struct VoyagerLayout;

#[cfg(feature = "palettefx")]
impl LedLayout for VoyagerLayout {
    fn count(&self) -> usize {
        LED_COUNT
    }

    fn position(&self, index: usize) -> (u8, u8) {
        LED_POS[index]
    }
}

/// Adafruit-style hue wheel: sweeps red -> green -> blue -> red.
#[cfg(not(feature = "palettefx"))]
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

#[cfg(not(feature = "palettefx"))]
fn scale(c: u8, brightness: u8) -> u8 {
    ((c as u16 * brightness as u16) / 255) as u8
}

/// Sentinel in `KEY_TO_LED` for a matrix cell with no physical LED.
pub const NO_LED: u8 = 0xFF;

/// Matrix (row, col) -> flat LED index into `LED_TABLE`, or `NO_LED`
/// for cells with no physical LED (unused matrix cells + the col-0
/// alpha row gaps). Derived from the same physical layout as `LED_X`
/// / `LED_Y`:
///
/// Left half:
///   rows 0-2 alphas        cols 1-6 → LEDs 0-5 / 6-11 / 12-17
///   row 3 alphas           cols 1-5 → LEDs 18-22
///   row 4 `B` drop         col 4   → LED 23
///   row 5 thumbs           cols 0-1 → LEDs 24-25
///
/// Right half:
///   rows 6-8 alphas        cols 0-5 → LEDs 26-31 / 32-37 / 38-43
///   row 9 alphas           cols 1-5 → LEDs 45-49
///   row 10 `N` drop        col 2   → LED 44
///   row 11 thumbs          cols 5-6 → LEDs 50-51
#[rustfmt::skip]
pub const KEY_TO_LED: [[u8; COL]; ROW] = [
    // Left half
    [NO_LED,  0,       1,      2,      3,      4,      5     ],
    [NO_LED,  6,       7,      8,      9,     10,     11     ],
    [NO_LED, 12,      13,     14,     15,     16,     17     ],
    [NO_LED, 18,      19,     20,     21,     22, NO_LED     ],
    [NO_LED, NO_LED, NO_LED, NO_LED,  23, NO_LED, NO_LED     ],
    [    24, 25,  NO_LED, NO_LED, NO_LED, NO_LED, NO_LED     ],
    // Right half
    [    26, 27,      28,     29,     30,     31, NO_LED     ],
    [    32, 33,      34,     35,     36,     37, NO_LED     ],
    [    38, 39,      40,     41,     42,     43, NO_LED     ],
    [NO_LED, 45,      46,     47,     48,     49, NO_LED     ],
    [NO_LED, NO_LED,  44, NO_LED, NO_LED, NO_LED, NO_LED     ],
    [NO_LED, NO_LED, NO_LED, NO_LED, NO_LED,     50,     51  ],
];

/// Look up the LED index for a matrix key press. Returns `None` for
/// out-of-range indices or matrix cells with no LED.
pub fn key_to_led(row: u8, col: u8) -> Option<u8> {
    let (r, c) = (row as usize, col as usize);
    if r >= ROW || c >= COL {
        return None;
    }
    match KEY_TO_LED[r][c] {
        NO_LED => None,
        idx => Some(idx),
    }
}

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

    pub fn set_all(&mut self, r: u8, g: u8, b: u8) {
        for entry in &LED_TABLE {
            let chip = entry.chip as usize;
            self.bufs[chip][entry.r as usize] = r;
            self.bufs[chip][entry.g as usize] = g;
            self.bufs[chip][entry.b as usize] = b;
            self.dirty[chip] = true;
        }
    }

    /// Write an rmk-palettefx HSV frame to the chip buffers. `frame[i]`
    /// is the colour for the i-th entry of `LED_TABLE`; each sample is
    /// converted to RGB via the rmk-palettefx spectrum converter.
    /// Global brightness is already baked into the HSV `v` field by
    /// `FrameParams::val`, so no further scaling happens here.
    #[cfg(feature = "palettefx")]
    pub fn paint_hsv(&mut self, frame: &[Hsv; LED_COUNT]) {
        for (idx, entry) in LED_TABLE.iter().enumerate() {
            let rgb = hsv_to_rgb(frame[idx]);
            let chip = entry.chip as usize;
            self.bufs[chip][entry.r as usize] = rgb.r;
            self.bufs[chip][entry.g as usize] = rgb.g;
            self.bufs[chip][entry.b as usize] = rgb.b;
            self.dirty[chip] = true;
        }
    }

    /// Horizontal rainbow: each key's hue is offset by its x position
    /// (8 hue units per column) plus `phase`. `brightness` scales the
    /// whole output.
    #[cfg(not(feature = "palettefx"))]
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
