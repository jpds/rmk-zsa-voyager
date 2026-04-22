#![no_std]
#![no_main]

#[macro_use]
mod macros;
mod keymap;

use core::ptr;

use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Speed};
use embassy_stm32::peripherals::USB;
use embassy_stm32::time::mhz;
use embassy_stm32::usb::{Driver, InterruptHandler};
use embassy_stm32::{Config, bind_interrupts};
use embassy_time::Timer;
use keymap::{COL, ROW};
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

/// Drive a status LED high whenever any non-base layer is active.
/// Subscribes to RMK's LayerChangeEvent; the current layer value is
/// published after every activate/deactivate/toggle.
#[embassy_executor::task]
async fn layer_indicator(mut led: Output<'static>) {
    let mut sub = LayerChangeEvent::subscriber();
    loop {
        let event = sub.next_event().await;
        if event.0 == 0 {
            led.set_low();
        } else {
            led.set_high();
        }
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

    // Status LED 2 (PB4) — lights up whenever a non-base layer is active.
    // Status LED 1 (PB5) is left idle for now.
    let layer_led = Output::new(p.PB4, Level::Low, Speed::Low);
    spawner.spawn(layer_indicator(layer_led).unwrap());

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

    // Left-half direct-GPIO matrix only. Right-half MCP23018 is step 4.
    // Polarity mirrors the stm32f1 example (active-HIGH strobe,
    // Pull::Down columns)
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

    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, ROW, COL, false>::new(row_pins, col_pins, debouncer);
    let mut keyboard = Keyboard::new(&keymap);

    join3(run_all!(matrix), keyboard.run(), run_rmk(driver, rmk_config)).await;
}
