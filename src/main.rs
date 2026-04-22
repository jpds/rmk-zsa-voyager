#![no_std]
#![no_main]

#[macro_use]
mod macros;
mod keymap;
mod mcp23018;

use core::ptr;

use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Speed};
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::peripherals::USB;
use embassy_stm32::time::{Hertz, mhz};
use embassy_stm32::usb::{Driver, InterruptHandler};
use embassy_stm32::{Config, bind_interrupts};
use embassy_time::Timer;
use keymap::{COL, ROW};
use core::sync::atomic::Ordering;

use mcp23018::{LED_PORTB, Mcp23018Matrix};
use panic_halt as _;
use rmk::config::{BehaviorConfig, DeviceConfig, PositionalConfig, RmkConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::event::{EventSubscriber, LayerChangeEvent, SubscribableEvent};
use rmk::futures::future::join3;
use rmk::input_device::Runnable;
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::{KeymapData, initialize_keymap, run_all, run_rmk};

bind_interrupts!(struct Irqs {
    USB_LP_CAN_RX0 => InterruptHandler<USB>;
});

/// Physical rows / cols on the left half (direct-GPIO scan).
const LEFT_ROWS: usize = 6;
const LEFT_COLS: usize = 7;

/// GD32F303 warm-boot cleanup for the ZSA Voyager.
/// The ZSA bootloader jumps to firmware without resetting the NVIC,
/// which can leave stale interrupts pending. Fix VTOR + clear all NVIC
/// enables/pending before embassy-stm32 takes over.
#[cortex_m_rt::pre_init]
unsafe fn pre_init() {
    unsafe {
        ptr::write_volatile(0xE000_ED08 as *mut u32, 0x0800_2000);

        core::arch::asm!("msr BASEPRI, {}", in(reg) 0u32);
        core::arch::asm!("cpsie i");
        core::arch::asm!("cpsie f");

        for i in 0..8u32 {
            ptr::write_volatile((0xE000_E180 + i * 4) as *mut u32, 0xFFFF_FFFF);
            ptr::write_volatile((0xE000_E280 + i * 4) as *mut u32, 0xFFFF_FFFF);
        }
    }
}

/// Apply a 4-bit LED frame:
///   bit 0 -> LED1 (PB5, direct GPIO, active high)
///   bit 1 -> LED2 (PB4, direct GPIO, active high)
///   bit 2 -> LED3 (MCP Port B bit 7, active low)
///   bit 3 -> LED4 (MCP Port B bit 6, active low)
fn apply_led_frame(led1: &mut Output<'static>, led2: &mut Output<'static>, bits: u8) {
    led1.set_level(((bits & 0b0001) != 0).into());
    led2.set_level(((bits & 0b0010) != 0).into());
    let led3_on = (bits >> 2) & 1;
    let led4_on = (bits >> 3) & 1;
    let portb = ((led3_on ^ 1) << 7) | ((led4_on ^ 1) << 6);
    LED_PORTB.store(portb, Ordering::Relaxed);
}

/// Boot LED cascade (500ms off, then 8x250ms frames lighting LED1..4
/// then clearing them in the same order) followed by a binary readout
/// of the currently active layer on every `LayerChangeEvent`.
#[embassy_executor::task]
async fn layer_indicator(mut led1: Output<'static>, mut led2: Output<'static>) {
    let mut sub = LayerChangeEvent::subscriber();

    const BOOT_FRAMES: [u8; 4] = [
        0b1001, 0b0110, 0b1111, 0b0000,
    ];
    Timer::after_millis(500).await;
    for &frame in &BOOT_FRAMES {
        apply_led_frame(&mut led1, &mut led2, frame);
        Timer::after_millis(250).await;
    }

    loop {
        let layer = sub.next_event().await.0;
        apply_led_frame(&mut led1, &mut led2, layer);
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hse = Some(Hse {
            freq: mhz(8),
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            src: PllSource::HSE,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL9,
        });
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2;
        config.rcc.apb2_pre = APBPrescaler::DIV1;
    }
    let p = embassy_stm32::init(config);

    // Status LEDs: PB5 = layer bit 0, PB4 = layer bit 1.
    let led_bit0 = Output::new(p.PB5, Level::Low, Speed::Low);
    let led_bit1 = Output::new(p.PB4, Level::Low, Speed::Low);
    spawner.spawn(layer_indicator(led_bit0, led_bit1).unwrap());

    // Warm-boot disconnect: the ZSA bootloader leaves its USB peripheral
    // active when jumping to firmware, so the host continues to see the
    // bootloader's D+ pull-up. CNTR.PDWN=1 / APB1RSTR toggle do NOT
    // release the pull-up on this hardware. What does work: gate the
    // USB clock off, drive PA12 (D+) low as a regular GPIO, hold 50ms
    // so the host sees a disconnect transient, restore PA12 to input,
    // then re-enable the USB clock. Driver::new then brings USB up
    // cleanly and the host enumerates our new device.
    unsafe {
        // RCC_APB1ENR = 0x4002_101C, USB clock gate = bit 23
        let apb1enr = 0x4002_101C as *mut u32;
        let v = ptr::read_volatile(apb1enr);
        ptr::write_volatile(apb1enr, v & !(1 << 23));

        // GPIOA_MODER = 0x48000000, PA12 mode in bits 25:24 (0b01 = output)
        let moder = 0x48000000 as *mut u32;
        let m = ptr::read_volatile(moder);
        ptr::write_volatile(moder, (m & !(0b11 << 24)) | (0b01 << 24));

        // GPIOA_ODR = 0x48000014, PA12 = 0
        let odr = 0x48000014 as *mut u32;
        let d = ptr::read_volatile(odr);
        ptr::write_volatile(odr, d & !(1 << 12));
    }
    Timer::after_millis(50).await;
    unsafe {
        // Restore PA12 MODER to input (0b00). USB peripheral will reclaim
        // the pin when re-enabled.
        let moder = 0x48000000 as *mut u32;
        let m = ptr::read_volatile(moder);
        ptr::write_volatile(moder, m & !(0b11 << 24));

        let apb1enr = 0x4002_101C as *mut u32;
        let v = ptr::read_volatile(apb1enr);
        ptr::write_volatile(apb1enr, v | (1 << 23));
    }

    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);

    // Deassert MCP23018 reset (PB8, active LOW) and let the chip settle
    // before the first I2C transaction.
    let _mcp_reset = Output::new(p.PB8, Level::High, Speed::Low);
    Timer::after_millis(10).await;

    // I2C1 on PB6 (SCL) / PB7 (SDA) at 400 kHz, blocking. The MCP matrix
    // driver retries init on NACK so a disconnected right half does not
    // block the left half.
    let mut i2c_config = i2c::Config::default();
    i2c_config.frequency = Hertz::khz(400);
    let mcp_i2c = I2c::new_blocking(p.I2C1, p.PB6, p.PB7, i2c_config);

    // Left-half direct-GPIO matrix. Scans rows 0-5 of the 12x7 keymap.
    let (col_pins, row_pins) = config_matrix_pins_stm32!(
        peripherals: p,
        input:  [PA0, PA1, PA2, PA3, PA6, PA7, PB0],
        output: [PB10, PB11, PB12, PB13, PB14, PB15]
    );

    let rmk_config = RmkConfig {
        device_config: DeviceConfig {
            manufacturer: "RMK",
            product_name: "ZSA Voyager",
            ..Default::default()
        },
        ..Default::default()
    };

    let mut keymap_data = KeymapData::new(keymap::get_default_keymap());
    let mut behavior_config = BehaviorConfig::default();
    let per_key_config = PositionalConfig::default();
    let keymap = initialize_keymap(&mut keymap_data, &mut behavior_config, &per_key_config).await;

    let left_debouncer = DefaultDebouncer::<LEFT_ROWS, LEFT_COLS>::new();
    let mut left_matrix =
        Matrix::<_, _, _, LEFT_ROWS, LEFT_COLS, false>::new(row_pins, col_pins, left_debouncer);

    let right_debouncer = DefaultDebouncer::<ROW, COL>::new();
    let mut right_matrix = Mcp23018Matrix::new(mcp_i2c, right_debouncer);

    let mut keyboard = Keyboard::new(&keymap);

    join3(
        run_all!(left_matrix, right_matrix),
        keyboard.run(),
        run_rmk(driver, rmk_config),
    )
    .await;
}
