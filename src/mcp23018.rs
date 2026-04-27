//! Right-half matrix driver via MCP23018 I/O expander.
//!
//! Port A bits 0-6 are row strobes (active LOW); Port B bits 0-5 are
//! column senses (pull-up, active LOW). Ports are wired opposite to
//! the logical matrix axes, so we transpose on the fly:
//!
//! ```text
//! logical row = 11 - sense_bit    // B0 -> row 11, B5 -> row 6
//! logical col = 6  - strobe_bit   // A0 -> col 6,  A6 -> col 0
//! ```

use core::sync::atomic::{AtomicU8, Ordering};

use embassy_stm32::i2c::{I2c, Master};
use embassy_stm32::mode::Blocking;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::{Mutex, MutexGuard};
use embassy_time::Timer;
use rmk::core_traits::Runnable;
use rmk::debounce::{DebounceState, DebouncerTrait};
use rmk::event::{KeyboardEvent, publish_event_async};
use rmk::matrix::KeyState;

pub type SharedI2c = Mutex<NoopRawMutex, I2c<'static, Blocking, Master>>;

/// Target byte for the two MCP-connected status LEDs. Bits 6/7 drive
/// the physical LED4 / LED3 outputs (active LOW). All other bits must
/// stay zero so we don't clobber the input pull-up OLAT state.
/// Written by the layer indicator task; flushed to Port B by the
/// matrix driver on the next scan pass when it changes.
pub static LED_PORTB: AtomicU8 = AtomicU8::new(0xC0);

const MCP_ADDR: u8 = 0x20;
const REG_IODIRA: u8 = 0x00;
const REG_IODIRB: u8 = 0x01;
const REG_GPPUA: u8 = 0x0C;
const REG_GPPUB: u8 = 0x0D;
const REG_GPIOA: u8 = 0x12;
const REG_GPIOB: u8 = 0x13;

const STROBES: usize = 7;
const SENSES: usize = 6;

/// Right-half matrix driver. Debouncer is sized for the full 12x7
/// keymap so that emitted events use the same coordinate space as the
/// left-half `Matrix`.
pub struct Mcp23018Matrix<'i, D: DebouncerTrait<12, 7>> {
    i2c: &'i SharedI2c,
    debouncer: D,
    key_states: [[KeyState; SENSES]; STROBES],
    initialized: bool,
    last_led_byte: u8,
}

impl<'i, D: DebouncerTrait<12, 7>> Mcp23018Matrix<'i, D> {
    pub fn new(i2c: &'i SharedI2c, debouncer: D) -> Self {
        Self {
            i2c,
            debouncer,
            key_states: [[KeyState::new(); SENSES]; STROBES],
            initialized: false,
            last_led_byte: 0xC0,
        }
    }

    fn init_expander_locked(
        &mut self,
        bus: &mut MutexGuard<'_, NoopRawMutex, I2c<'static, Blocking, Master>>,
    ) -> bool {
        let cfg: [[u8; 2]; 5] = [
            [REG_IODIRA, 0x00],
            [REG_IODIRB, 0x3F],
            [REG_GPPUA, 0x00],
            [REG_GPPUB, 0x3F],
            // Park status LEDs on B6/B7 off (active low).
            [REG_GPIOB, 0xC0],
        ];
        for cmd in &cfg {
            if bus.blocking_write(MCP_ADDR, cmd).is_err() {
                return false;
            }
        }
        self.last_led_byte = 0xC0;
        true
    }

    fn sync_leds_locked(
        &mut self,
        bus: &mut MutexGuard<'_, NoopRawMutex, I2c<'static, Blocking, Master>>,
    ) {
        let target = LED_PORTB.load(Ordering::Relaxed);
        if target == self.last_led_byte {
            return;
        }
        if bus.blocking_write(MCP_ADDR, &[REG_GPIOB, target]).is_ok() {
            self.last_led_byte = target;
        } else {
            self.initialized = false;
        }
    }

    async fn scan_once(&mut self) {
        let mut bus = self.i2c.lock().await;
        self.sync_leds_locked(&mut bus);
        if !self.initialized {
            return;
        }
        for strobe in 0..STROBES {
            let a_val: u8 = 0x7F & !(1u8 << strobe);
            if bus.blocking_write(MCP_ADDR, &[REG_GPIOA, a_val]).is_err() {
                self.initialized = false;
                return;
            }
            // Release the bus so the RGB painter can slip in during the
            // row settle window, then re-acquire to read.
            drop(bus);
            Timer::after_micros(10).await;
            bus = self.i2c.lock().await;

            let mut rx = [0u8; 1];
            if bus
                .blocking_write_read(MCP_ADDR, &[REG_GPIOB], &mut rx)
                .is_err()
            {
                self.initialized = false;
                return;
            }
            let gpiob = rx[0];

            for sense in 0..SENSES {
                let active = (gpiob & (1u8 << sense)) == 0;
                let row = 11 - sense;
                let col = 6 - strobe;
                let prev = self.key_states[strobe][sense];
                let debounce = self
                    .debouncer
                    .detect_change_with_debounce(row, col, active, &prev);
                if let DebounceState::Debounced = debounce {
                    self.key_states[strobe][sense].toggle_pressed();
                    let pressed = self.key_states[strobe][sense].pressed;
                    publish_event_async(KeyboardEvent::key(row as u8, col as u8, pressed)).await;
                }
            }
        }
    }
}

impl<'i, D: DebouncerTrait<12, 7>> Runnable for Mcp23018Matrix<'i, D> {
    async fn run(&mut self) -> ! {
        loop {
            if !self.initialized {
                let mut bus = self.i2c.lock().await;
                let ok = self.init_expander_locked(&mut bus);
                drop(bus);
                if ok {
                    self.initialized = true;
                } else {
                    Timer::after_millis(100).await;
                    continue;
                }
            }
            self.scan_once().await;
            Timer::after_micros(500).await;
        }
    }
}
