//! IS31FL3731 LED driver — initial bringup.
//!
//! The Voyager has two IS31FL3731 chips on the shared I2C bus:
//!   - 0x74 (ADDR pin to GND) — left half, 26 RGB LEDs
//!   - 0x77 (ADDR pin to VCC) — right half, 26 RGB LEDs
//!
//! Per-LED colors require the register map that associates each key's
//! R/G/B channels with specific PWM register addresses. For the first
//! bringup we just write the same PWM value to every register on both
//! chips, producing a uniform white glow under every switch.

use embassy_stm32::i2c::{I2c, Master};
use embassy_stm32::mode::Blocking;
use embassy_time::Timer;

pub const ADDR_LEFT: u8 = 0x74;
pub const ADDR_RIGHT: u8 = 0x77;

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

const LED_CONTROL_COUNT: u8 = 18;
const PWM_CHUNKS: u8 = 9;
const CHUNK_BYTES: usize = 16;

fn write_reg(i2c: &mut I2c<'_, Blocking, Master>, addr: u8, reg: u8, val: u8) -> Result<(), ()> {
    i2c.blocking_write(addr, &[reg, val]).map_err(|_| ())
}

fn select_page(i2c: &mut I2c<'_, Blocking, Master>, addr: u8, page: u8) -> Result<(), ()> {
    write_reg(i2c, addr, REG_COMMAND, page)
}

/// Initialize one IS31FL3731 and paint every PWM register with `pwm`.
/// Every physical LED on that half will glow at that brightness.
pub async fn init_solid(
    i2c: &mut I2c<'_, Blocking, Master>,
    addr: u8,
    pwm: u8,
) -> Result<(), ()> {
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

    // Burst 16 bytes of PWM data per transfer, 9 chunks = 144 regs.
    let mut buf = [0u8; 1 + CHUNK_BYTES];
    buf[1..].fill(pwm);
    for chunk in 0..PWM_CHUNKS {
        buf[0] = FRAME_REG_PWM + chunk * CHUNK_BYTES as u8;
        i2c.blocking_write(addr, &buf).map_err(|_| ())?;
    }

    select_page(i2c, addr, CMD_FUNCTION)?;
    write_reg(i2c, addr, FN_SHUTDOWN, 0x01)?;
    select_page(i2c, addr, CMD_FRAME_1)?;
    Ok(())
}
