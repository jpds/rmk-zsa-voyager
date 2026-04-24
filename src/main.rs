#![no_std]
#![no_main]

#[macro_use]
mod macros;
mod is31fl3731;
mod keymap;
mod mcp23018;
mod vial;

use core::ptr;
use core::sync::atomic::Ordering;

use embassy_executor::Spawner;
use embassy_stm32::flash::Flash;
use embassy_stm32::gpio::{Input, Level, Output, Speed};
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::peripherals::USB;
use embassy_stm32::time::{Hertz, mhz};
use embassy_stm32::usb::{Driver, InterruptHandler};
use embassy_stm32::{Config, bind_interrupts};
use embassy_sync::mutex::Mutex;
#[cfg(feature = "palettefx")]
use embassy_time::Instant;
use embassy_time::Timer;
#[cfg(feature = "palettefx")]
use is31fl3731::{LED_COUNT, VoyagerLayout};
use is31fl3731::Rgb;
use keymap::{COL, ROW};
use mcp23018::{LED_PORTB, Mcp23018Matrix, SharedI2c};
use panic_halt as _;
use rmk::config::{
    BehaviorConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig,
};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::event::{
    ActionEvent, EventSubscriber, LayerChangeEvent,
    SubscribableEvent,
};
#[cfg(feature = "palettefx")]
use rmk::event::{KeyboardEvent, KeyboardEventPos};
use rmk::types::action::Action;
use rmk::types::keycode::{HidKeyCode, KeyCode};
use rmk::types::morse::{Morse, MorseProfile};
#[cfg(feature = "palettefx")]
use rmk::types::action::LightAction;
use rmk::futures::future::join4;
use rmk::input_device::Runnable;
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::storage::async_flash_wrapper;
use rmk::{KeymapData, initialize_keymap_and_storage, run_all, run_rmk};
#[cfg(feature = "palettefx")]
use rmk_palettefx::color::Hsv;
#[cfg(feature = "palettefx")]
use rmk_palettefx::effects::{
    FlowState, FrameParams, Pcg32, ReactiveState, RippleState, SparkleState, VortexState, gradient,
};
#[cfg(feature = "palettefx")]
use rmk_palettefx::layout::LedLayout;
#[cfg(feature = "palettefx")]
use rmk_palettefx::palette::{BUILTIN_PALETTES, Palette, id};
use vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};

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
    led1.set_level(((bits & 0b0001) != 0).into());
    led2.set_level(((bits & 0b0010) != 0).into());
    let led3_on = (bits >> 2) & 1;
    let led4_on = (bits >> 3) & 1;
    let portb = ((led3_on ^ 1) << 7) | ((led4_on ^ 1) << 6);
    LED_PORTB.store(portb, Ordering::Relaxed);
}

/// Initial brightness. With palettefx this feeds `FrameParams::val`;
/// without it scales the rainbow wheel output.
const RGB_BRIGHTNESS: u8 = 0x30;
#[cfg(feature = "palettefx")]
const MIN_VAL: u8 = 0x08;
/// Step applied by a single `light!(RgbVai/RgbVad)` press.
#[cfg(feature = "palettefx")]
const VAL_STEP: u8 = 0x10;

/// Initial animation speed (FrameParams::speed). Adjusted by
/// `light!(RgbSpi/RgbSpd)` at runtime. 192 was tuned on hardware.
#[cfg(feature = "palettefx")]
const RGB_SPEED: u8 = 192;
#[cfg(feature = "palettefx")]
const SPEED_STEP: u8 = 16;

/// Number of simultaneous key-hits the Reactive effect remembers.
/// Each hit fades out over ~1.3 s at the default speed, so 16 comfortably
/// covers sustained ~8-10 keys-per-second typing.
#[cfg(feature = "palettefx")]
const REACTIVE_HITS: usize = 16;

/// Fresh `Pcg32` state for Ripple. Constructed each time Ripple is
/// cycled to so the drop sequence restarts deterministically.
#[cfg(feature = "palettefx")]
fn fresh_ripple_rng() -> Pcg32 {
    Pcg32::new(0xDEAD_C0DE_CAFE_F00D, 0xA02B_DBF7_BB3C_0A7F)
}

/// Dynamically-switchable rmk-palettefx effect for the base layer.
/// Each variant owns its own state so cycling resets the time phase.
/// Gradient is stateless. Ripple carries a Pcg32 for drop-placement
/// randomness; Reactive carries a ring of recent key hits.
#[cfg(feature = "palettefx")]
enum Effect {
    Gradient,
    Flow(FlowState),
    Vortex(VortexState),
    Sparkle(SparkleState),
    Ripple(RippleState, Pcg32),
    Reactive(ReactiveState<REACTIVE_HITS>),
}

#[cfg(feature = "palettefx")]
impl Effect {
    fn tick(&mut self, params: FrameParams<'_>, out: &mut [Hsv; LED_COUNT]) {
        match self {
            Self::Gradient => gradient(&VoyagerLayout, params, out),
            Self::Flow(s) => s.tick(&VoyagerLayout, params, out),
            Self::Vortex(s) => s.tick(&VoyagerLayout, params, out),
            Self::Sparkle(s) => s.tick(&VoyagerLayout, params, out),
            Self::Ripple(s, rng) => s.tick_with_rng(rng, &VoyagerLayout, params, out),
            Self::Reactive(s) => s.tick(&VoyagerLayout, params, out),
        }
    }

    fn next(&mut self) {
        *self = match self {
            Self::Gradient => Self::Flow(FlowState::new()),
            Self::Flow(_) => Self::Vortex(VortexState::new()),
            Self::Vortex(_) => Self::Sparkle(SparkleState::new()),
            Self::Sparkle(_) => Self::Ripple(RippleState::new(), fresh_ripple_rng()),
            Self::Ripple(_, _) => Self::Reactive(ReactiveState::new()),
            Self::Reactive(_) => Self::Gradient,
        };
    }

    fn prev(&mut self) {
        *self = match self {
            Self::Gradient => Self::Reactive(ReactiveState::new()),
            Self::Reactive(_) => Self::Ripple(RippleState::new(), fresh_ripple_rng()),
            Self::Ripple(_, _) => Self::Sparkle(SparkleState::new()),
            Self::Sparkle(_) => Self::Vortex(VortexState::new()),
            Self::Vortex(_) => Self::Flow(FlowState::new()),
            Self::Flow(_) => Self::Gradient,
        };
    }

    /// Record a key press against the Reactive effect; no-op for every
    /// other variant. Returns `true` iff a hit was actually recorded,
    /// so the caller can skip the follow-up re-tick / flush when it
    /// wouldn't be visible.
    fn record_hit(&mut self, led_idx: usize, timer_ms: u32) -> bool {
        let Self::Reactive(s) = self else {
            return false;
        };
        let (x, y) = VoyagerLayout.position(led_idx);
        s.record_hit(x, y, timer_ms);
        true
    }
}

/// Paint a non-base layer's solid palette. Base layer (0) is animated
/// in the main tick loop instead of being a static paint.
fn paint_static_layer(rgb: &mut Rgb, layer: u8) {
    match layer {
        1 => rgb.set_all(0x00, 0x10, 0x40), // symbols/F-keys: cool blue
        2 => rgb.set_all(0x30, 0x00, 0x30), // media/nav: magenta
        _ => rgb.set_all(0x20, 0x20, 0x20),
    }
}

/// Tick the active rmk-palettefx effect with the current palette /
/// brightness / speed, then write the resulting HSV frame into the
/// chip buffers. Called from the layer-0 animation path below and on
/// every RGB state change so the LEDs respond immediately.
#[cfg(feature = "palettefx")]
fn tick_base_effect(
    effect: &mut Effect,
    palette: &Palette,
    val: u8,
    speed: u8,
    frame: &mut [Hsv; LED_COUNT],
    rgb: &mut Rgb,
) {
    effect.tick(
        FrameParams {
            palette,
            speed,
            sat: 255,
            val,
            timer_ms: Instant::now().as_millis() as u32,
        },
        frame,
    );
    rgb.paint_hsv(frame);
}

/// Drive the four status LEDs (4-bit binary counter of the highest
/// active layer), animate the per-key RGB matrix when the base layer
/// is active, and swap to a solid per-layer palette for other layers.
///
/// Boot behavior: 500 ms off, then an 8x250 ms cascade lighting LED1..4
/// and clearing them in the same order. The Flow animation takes over
/// once the cascade finishes.
async fn layer_indicator(
    led1: &mut Output<'static>,
    led2: &mut Output<'static>,
    i2c: &SharedI2c,
) -> ! {
    use rmk::embassy_futures::select::{Either4, select4};

    let mut layer_sub = LayerChangeEvent::subscriber();
    let mut action_sub = ActionEvent::subscriber();
    // KeyboardEvent feeds the Reactive effect's per-key hit history.
    // It's a no-op for other effects; `record_hit` gates the flush.
    #[cfg(feature = "palettefx")]
    let mut key_sub = KeyboardEvent::subscriber();
    let mut rgb = Rgb::new();
    #[cfg(feature = "palettefx")]
    let mut effect = Effect::Flow(FlowState::new());
    #[cfg(feature = "palettefx")]
    let mut palette_idx: usize = id::AFTERBURN;
    #[cfg(feature = "palettefx")]
    let mut val: u8 = RGB_BRIGHTNESS;
    #[cfg(feature = "palettefx")]
    let mut speed: u8 = RGB_SPEED;
    #[cfg(feature = "palettefx")]
    let mut hsv_frame = [Hsv::default(); LED_COUNT];
    #[cfg(not(feature = "palettefx"))]
    let mut phase: u8 = 0;

    const BOOT_FRAMES: [u8; 4] = [
        0b1001, 0b0110, 0b1111, 0b0000,
    ];
    Timer::after_millis(500).await;
    for &frame in &BOOT_FRAMES {
        apply_led_frame(led1, led2, frame);
        Timer::after_millis(250).await;
    }

    let mut layer: u8 = 0;
    #[cfg(feature = "palettefx")]
    tick_base_effect(
        &mut effect,
        BUILTIN_PALETTES[palette_idx],
        val,
        speed,
        &mut hsv_frame,
        &mut rgb,
    );
    #[cfg(not(feature = "palettefx"))]
    rgb.paint_rainbow(phase, RGB_BRIGHTNESS);
    {
        let mut bus = i2c.lock().await;
        let _ = rgb.flush(&mut bus);
    }

    loop {
        let tick = Timer::after_millis(50);
        match select4(
            tick,
            layer_sub.next_event(),
            action_sub.next_event(),
            #[cfg(feature = "palettefx")]
            key_sub.next_event(),
            #[cfg(not(feature = "palettefx"))]
            core::future::pending::<()>(),
        )
        .await
        {
            Either4::First(_) => {
                #[cfg(feature = "palettefx")]
                if layer == 0 {
                    tick_base_effect(
                        &mut effect,
                        BUILTIN_PALETTES[palette_idx],
                        val,
                        speed,
                        &mut hsv_frame,
                        &mut rgb,
                    );
                    let mut bus = i2c.lock().await;
                    let _ = rgb.flush(&mut bus);
                }
                #[cfg(not(feature = "palettefx"))]
                if layer == 0 {
                    phase = phase.wrapping_add(2);
                    rgb.paint_rainbow(phase, RGB_BRIGHTNESS);
                    let mut bus = i2c.lock().await;
                    let _ = rgb.flush(&mut bus);
                }
            }
            Either4::Second(event) => {
                layer = event.0;
                apply_led_frame(led1, led2, layer);
                #[cfg(feature = "palettefx")]
                if layer == 0 {
                    tick_base_effect(
                        &mut effect,
                        BUILTIN_PALETTES[palette_idx],
                        val,
                        speed,
                        &mut hsv_frame,
                        &mut rgb,
                    );
                } else {
                    paint_static_layer(&mut rgb, layer);
                }
                #[cfg(not(feature = "palettefx"))]
                if layer == 0 {
                    rgb.paint_rainbow(phase, RGB_BRIGHTNESS);
                } else {
                    paint_static_layer(&mut rgb, layer);
                }
                let mut bus = i2c.lock().await;
                let _ = rgb.flush(&mut bus);
            }
            Either4::Third(event) => {
                #[cfg(feature = "palettefx")]
                {
                    let Action::Light(light) = event.action else {
                        continue;
                    };
                    // rmk publishes ActionEvent on both press and release;
                    // acting on either would apply each adjustment twice.
                    // Act on press only.
                    if !event.keyboard_event.pressed {
                        continue;
                    }
                    let n = BUILTIN_PALETTES.len();
                    match light {
                        LightAction::RgbModeForward => effect.next(),
                        LightAction::RgbModeReverse => effect.prev(),
                        LightAction::RgbHui => palette_idx = (palette_idx + 1) % n,
                        LightAction::RgbHud => palette_idx = (palette_idx + n - 1) % n,
                        LightAction::RgbVai => val = val.saturating_add(VAL_STEP),
                        LightAction::RgbVad => val = val.saturating_sub(VAL_STEP).max(MIN_VAL),
                        LightAction::RgbSpi => speed = speed.saturating_add(SPEED_STEP),
                        LightAction::RgbSpd => speed = speed.saturating_sub(SPEED_STEP),
                        _ => continue,
                    }
                    if layer == 0 {
                        tick_base_effect(
                            &mut effect,
                            BUILTIN_PALETTES[palette_idx],
                            val,
                            speed,
                            &mut hsv_frame,
                            &mut rgb,
                        );
                        let mut bus = i2c.lock().await;
                        let _ = rgb.flush(&mut bus);
                    }
                }
            }
            Either4::Fourth(_event) => {
                #[cfg(feature = "palettefx")]
                {
                    // Feed the Reactive effect. Other variants don't care,
                    // and `record_hit` short-circuits to a no-op for them.
                    if !_event.pressed || layer != 0 {
                        continue;
                    }
                    let KeyboardEventPos::Key(pos) = _event.pos else {
                        continue;
                    };
                    let Some(led) = is31fl3731::key_to_led(pos.row, pos.col) else {
                        continue;
                    };
                    let now_ms = Instant::now().as_millis() as u32;
                    if effect.record_hit(led as usize, now_ms) {
                        // Reactive recorded the hit; paint immediately so
                        // the user sees a response on the press rather
                        // than up to 50 ms later on the next tick.
                        tick_base_effect(
                            &mut effect,
                            BUILTIN_PALETTES[palette_idx],
                            val,
                            speed,
                            &mut hsv_frame,
                            &mut rgb,
                        );
                        let mut bus = i2c.lock().await;
                        let _ = rgb.flush(&mut bus);
                    }
                }
            }
        }
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
    // the layer_indicator future (joined below) can drive them.
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
    // the layer indicator's RGB flush path (on layer change) via a
    // NoopRawMutex; the mutex lives in main's stack frame and is
    // referenced by both futures joined below.
    let mut i2c_config = i2c::Config::default();
    i2c_config.frequency = Hertz::khz(400);
    let shared_i2c: SharedI2c = Mutex::new(I2c::new_blocking(p.I2C1, p.PB6, p.PB7, i2c_config));

    // Bring both IS31FL3731 chips out of shutdown. Failures are
    // swallowed; the keyboard still enumerates and scans if RGB is
    // unresponsive.
    {
        let mut bus = shared_i2c.lock().await;
        let _ = is31fl3731::init_chip(&mut bus, is31fl3731::ADDR_LEFT).await;
        let _ = is31fl3731::init_chip(&mut bus, is31fl3731::ADDR_RIGHT).await;
    }

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
        vial_config: VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF, &[]),
        ..Default::default()
    };

    // Internal flash for Vial keymap persistence. StorageConfig::default()
    // parks storage in the last two flash sectors; memory.x reserves that
    // range so the linker never places firmware there.
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

    join4(
        run_all!(left_matrix, right_matrix),
        layer_indicator(&mut led_bit0, &mut led_bit1, &shared_i2c),
        keyboard.run(),
        run_rmk(&keymap, driver, &mut storage, rmk_config),
    )
    .await;
}
