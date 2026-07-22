#![no_std]
#![no_main]

#[macro_use]
mod macros;
mod is31fl3731;
mod keymap;
mod lighting;
mod mcp23018;

use core::ptr;

use embassy_executor::Spawner;
use embassy_stm32::flash::Flash;
use embassy_stm32::gpio::{Input, Level, Output, Speed};
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::peripherals::USB;
use embassy_stm32::time::{Hertz, mhz};
use embassy_stm32::usb::{Driver, InterruptHandler};
use embassy_stm32::{Config, bind_interrupts};
use embassy_sync::mutex::Mutex;
use embassy_time::{Instant, Timer};
use keymap::{COL, ROW};
use mcp23018::{Mcp23018Matrix, SharedI2c};
use panic_halt as _;
use rmk::config::{BehaviorConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig};
use rmk::core_traits::Runnable;
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::event::{
    EventSubscriber, KeyboardEvent, KeyboardEventPos, LayerChangeEvent, SubscribableEvent,
};
use rmk::futures::future::join5;
#[cfg(feature = "rynk")]
use rmk::host::HostService;
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::storage::async_flash_wrapper;
use rmk::types::action::Action;
use rmk::types::keycode::{HidKeyCode, KeyCode};
use rmk::types::morse::{Morse, MorseProfile};
#[cfg(feature = "rynk")]
use rmk::types::protocol::rynk::LightingSceneCell;
use rmk::usb::UsbTransport;
use rmk::watchdog::{WatchdogFeed, WatchdogRunner};
use rmk::{KeymapData, initialize_keymap_and_storage, run_all};

// Physical layout, lighting topology, and layout blob statics from
// keyboard.toml: LIGHTING_TOPOLOGY, LIGHTING_ROUTING, LIGHTING_LAYER_SCENES,
// LIGHTING_CONTROLS, LIGHTING_BACKGROUND, LAYOUT_BLOB, ...
rmk::macros::rmk_lighting_config!();

const IWDG_KR: *mut u32 = 0x4000_3000 as *mut u32;
const IWDG_PR: *mut u32 = 0x4000_3004 as *mut u32;
const IWDG_RLR: *mut u32 = 0x4000_3008 as *mut u32;

fn iwdg_start() {
    unsafe {
        ptr::write_volatile(IWDG_KR, 0xCCCC);
        ptr::write_volatile(IWDG_KR, 0x5555);
        ptr::write_volatile(IWDG_PR, 0b110); // /256 → ~125 Hz ticks at LSI 32 kHz
        ptr::write_volatile(IWDG_RLR, 1250); // 1250 / 125 Hz = 10s timeout
    }
}

struct Stm32Iwdg;

impl WatchdogFeed for Stm32Iwdg {
    fn feed(&mut self) {
        unsafe {
            ptr::write_volatile(IWDG_KR, 0xAAAA);
        }
    }
}

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

/// Apply a 4-bit status-LED frame:
///   bit 0 -> LED1 (PB5, direct GPIO, active high)
///   bit 1 -> LED2 (PB4, direct GPIO, active high)
///   bit 2 -> LED3 (MCP Port B bit 7, active low)
///   bit 3 -> LED4 (MCP Port B bit 6, active low)
fn apply_led_frame(led1: &mut Output<'static>, led2: &mut Output<'static>, bits: u8) {
    use core::sync::atomic::Ordering;

    led1.set_level(((bits & 0b0001) != 0).into());
    led2.set_level(((bits & 0b0010) != 0).into());
    let led3_on = (bits >> 2) & 1;
    let led4_on = (bits >> 3) & 1;
    let portb = ((led3_on ^ 1) << 7) | ((led4_on ^ 1) << 6);
    mcp23018::LED_PORTB.store(portb, Ordering::Relaxed);
}

/// Drive the four status LEDs as a 4-bit binary counter of the highest
/// active layer. The per-key RGB matrix is owned by the lighting engine
/// (see lighting.rs); this task only touches the discrete LEDs.
///
/// Boot behavior: 500 ms off, then an 8x250 ms cascade lighting LED1..4
/// and clearing them in the same order.
async fn status_leds(led1: &mut Output<'static>, led2: &mut Output<'static>) -> ! {
    let mut layer_sub = LayerChangeEvent::subscriber();

    const BOOT_FRAMES: [u8; 4] = [0b1001, 0b0110, 0b1111, 0b0000];
    Timer::after_millis(500).await;
    for &frame in &BOOT_FRAMES {
        apply_led_frame(led1, led2, frame);
        Timer::after_millis(250).await;
    }

    loop {
        let event = layer_sub.next_event().await;
        apply_led_frame(led1, led2, event.0);
    }
}

/// Feed key presses to the Reactive palettefx effect. `record_key_hit` is
/// a no-op render-wise unless Reactive is the active effect (the source
/// drains the queue either way).
async fn reactive_key_hits() -> ! {
    let mut key_sub = KeyboardEvent::subscriber();
    loop {
        let event = key_sub.next_event().await;
        if !event.pressed {
            continue;
        }
        let KeyboardEventPos::Key(pos) = event.pos else {
            continue;
        };
        let Some(led) = is31fl3731::key_to_led(pos.row, pos.col) else {
            continue;
        };
        lighting::record_key_hit(led, Instant::now().as_millis() as u32);
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
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

    // Status LEDs: PB5 = layer bit 0, PB4 = layer bit 1. Held here so
    // the status_leds future (joined below) can drive them.
    let mut led_bit0 = Output::new(p.PB5, Level::Low, Speed::Low);
    let mut led_bit1 = Output::new(p.PB4, Level::Low, Speed::Low);

    // Warm-boot disconnect: the ZSA bootloader leaves its USB peripheral
    // active when jumping to firmware, so the host continues to see the
    // bootloader's D+ pull-up. CNTR.PDWN=1 / APB1RSTR toggle do NOT
    // release the pull-up on this hardware. What does work: gate the
    // USB clock off and drive PA12 (D+) low as a regular GPIO. We hold
    // that state all the way through storage init (see the matching
    // restore just before Driver::new below); otherwise a slow
    // clear_storage erase pushes SET_ADDRESS past the host's timeout.
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
    // PA12 stays driven low as GPIO and the USB clock stays off until
    // just before Driver::new below. Re-enabling the clock here would
    // bring the internal pull-up back up (the bootloader leaves PDWN=0,
    // so clocking alone is enough to reassert D+), starting the host's
    // enumeration timer before storage init has completed. Holding the
    // disconnect through storage init keeps SET_ADDRESS inside the
    // host's window.

    // Deassert MCP23018 reset (PB8, active LOW) and let the chip settle
    // before the first I2C transaction.
    let _mcp_reset = Output::new(p.PB8, Level::High, Speed::Low);
    Timer::after_millis(10).await;

    // I2C1 on PB6 (SCL) / PB7 (SDA) at 400 kHz, blocking. The bus is
    // shared between the MCP matrix driver (continuous scanning) and
    // the lighting output's flush path via a NoopRawMutex; the mutex
    // lives in main's stack frame and is referenced by both futures
    // joined below. The IS31FL3731 chips themselves are initialized by
    // the lighting service (Is31Output::initialize).
    let mut i2c_config = i2c::Config::default();
    i2c_config.frequency = Hertz::khz(400);
    let shared_i2c: SharedI2c = Mutex::new(I2c::new_blocking(p.I2C1, p.PB6, p.PB7, i2c_config));

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
        #[cfg(feature = "rynk")]
        layout_blob: LAYOUT_BLOB,
        ..Default::default()
    };

    // Internal flash for keymap + lighting-scene persistence.
    // StorageConfig::default() parks storage in the last two flash
    // sectors; memory.x reserves that range so the linker never places
    // firmware there.
    let flash = async_flash_wrapper(Flash::new_blocking(p.FLASH));
    let storage_config = StorageConfig::default();

    let mut keymap_data = KeymapData::new(keymap::get_default_keymap());
    let mut behavior_config = BehaviorConfig::default();
    // TD_ESC_EQL: tap = Escape, double-tap = Equal.
    let _ = behavior_config.morse.morses.push(Morse::new_from_vial(
        Action::Key(KeyCode::Hid(HidKeyCode::Escape)),
        Action::No,
        Action::No,
        Action::Key(KeyCode::Hid(HidKeyCode::Equal)),
        MorseProfile::const_default(),
    ));
    let per_key_config = PositionalConfig::new(keymap::HAND_MAP);
    let (keymap, mut storage) = initialize_keymap_and_storage(
        &mut keymap_data,
        flash,
        &storage_config,
        &mut behavior_config,
        &per_key_config,
    )
    .await;

    let left_debouncer = DefaultDebouncer::<LEFT_ROWS, LEFT_COLS>::new();
    let mut left_matrix =
        Matrix::<_, _, _, LEFT_ROWS, LEFT_COLS, false>::new(row_pins, col_pins, left_debouncer);

    let right_debouncer = DefaultDebouncer::<ROW, COL>::new();
    let mut right_matrix = Mcp23018Matrix::new(&shared_i2c, right_debouncer);

    let mut keyboard = Keyboard::new(&keymap);

    // Lighting: restore Rynk-persisted scenes, then hand the engine, the
    // IS31FL3731 output, and the Rynk bridge to their runnables.
    #[cfg(feature = "rynk")]
    let mut persisted_scenes =
        rmk::heapless::Vec::<LightingSceneCell, { lighting::SCENE_CAPACITY }>::new();
    #[cfg(feature = "rynk")]
    let persisted_policy = storage.read_lighting_scenes(&mut persisted_scenes).await;
    let mut lighting_processor = lighting::init(
        &keymap,
        #[cfg(feature = "rynk")]
        persisted_scenes.as_slice(),
        #[cfg(feature = "rynk")]
        persisted_policy,
        &shared_i2c,
    );
    #[cfg(feature = "rynk")]
    let mut rynk_lighting_adapter = lighting::rynk_adapter();

    #[cfg(feature = "rynk")]
    let host_service =
        HostService::new(&keymap, &rmk_config).with_lighting(lighting::rynk_controller());

    // Storage init is done; release the warm-boot disconnect and hand
    // PA12/USB back to the peripheral. The host sees D+ come up only
    // now, well after any clear_storage flash erase has finished, so
    // enumeration starts against a device that can respond immediately.
    unsafe {
        // Restore PA12 MODER to input (0b00).
        let moder = 0x48000000 as *mut u32;
        let m = ptr::read_volatile(moder);
        ptr::write_volatile(moder, m & !(0b11 << 24));

        // Re-enable USB clock.
        let apb1enr = 0x4002_101C as *mut u32;
        let v = ptr::read_volatile(apb1enr);
        ptr::write_volatile(apb1enr, v | (1 << 23));
    }
    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);

    iwdg_start();
    let mut watchdog_runner =
        WatchdogRunner::new(Stm32Iwdg, embassy_time::Duration::from_secs(5));

    #[cfg(feature = "rynk")]
    let mut usb_transport =
        UsbTransport::new(driver, rmk_config.device_config).with_host_service(&host_service);
    #[cfg(not(feature = "rynk"))]
    let mut usb_transport = UsbTransport::new(driver, rmk_config.device_config);

    #[cfg(feature = "rynk")]
    join5(
        run_all!(
            left_matrix,
            right_matrix,
            storage,
            watchdog_runner,
            lighting_processor,
            rynk_lighting_adapter
        ),
        status_leds(&mut led_bit0, &mut led_bit1),
        reactive_key_hits(),
        keyboard.run(),
        usb_transport.run(),
    )
    .await;
    #[cfg(not(feature = "rynk"))]
    join5(
        run_all!(
            left_matrix,
            right_matrix,
            storage,
            watchdog_runner,
            lighting_processor
        ),
        status_leds(&mut led_bit0, &mut led_bit1),
        reactive_key_hits(),
        keyboard.run(),
        usb_transport.run(),
    )
    .await;
}
