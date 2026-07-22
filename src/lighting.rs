//! RMK lighting-abstraction wiring: the rmk-palettefx extension source, the
//! IS31FL3731 output, and (with the `rynk` feature) the Rynk lighting bridge.
//!
//! The standard engine owns composition (background < extension < layer
//! scenes < host overlay < status). The palettefx animation lives in the
//! extension band, so the layer-1/2 solid washes declared in keyboard.toml
//! opaquely cover it — and erase its render deadlines — whenever a layer is
//! held, exactly like the old hand-rolled `layer_indicator` behaviour.

use core::num::NonZeroU32;

#[cfg(feature = "rynk")]
use rmk::host::{
    RynkLightingController, RynkLightingDescriptor, RynkLightingMailbox,
    StandardRynkLightingAdapter, install_lighting_scenes,
};
use rmk::keymap::KeyMap;
use rmk::lighting::{
    EmptySource, KeymapLightingState, LightingMailbox, LightingOutput, LightingProcessor,
    LightingService, LogicalFrame, OutputOperation, Rgb8, StandardCommand, StandardError,
    StandardLightingEngine, StandardReply,
};
#[cfg(feature = "rynk")]
use rmk::types::protocol::rynk::{LightingLayerPolicy, LightingSceneCell};
use rmk_palettefx::palette::id;
use rmk_palettefx::rmk_lighting::{HitQueue, PaletteFxConfig, PaletteFxSource};

use crate::is31fl3731::{self, ADDR_LEFT, ADDR_RIGHT, LED_COUNT, Rgb, VoyagerLayout};
use crate::mcp23018::SharedI2c;

// Sized for the 32 KB-RAM STM32F303: StandardCommand inlines a full replica
// state (overlay + scene tables), so these capacities set the mailbox entry
// size as well as the engine tables. 16 concurrent host-overlay cells and 32
// persisted scene cells are plenty for 52 LEDs.
pub const OVERLAY_CAPACITY: usize = 16;
pub const SCENE_CAPACITY: usize = 32;
pub const COMMAND_CAPACITY: usize = 2;

/// Number of simultaneous key-hits the Reactive effect remembers. Each hit
/// fades out over ~1.3 s at the default speed, so 16 comfortably covers
/// sustained ~8-10 keys-per-second typing.
const REACTIVE_HITS: usize = 16;

pub type Engine = StandardLightingEngine<
    'static,
    PaletteFxSource<VoyagerLayout, LED_COUNT, REACTIVE_HITS>,
    EmptySource,
    LED_COUNT,
    OVERLAY_CAPACITY,
    SCENE_CAPACITY,
>;
pub type CoreMailbox = LightingMailbox<
    StandardCommand<OVERLAY_CAPACITY, SCENE_CAPACITY>,
    StandardReply,
    StandardError,
    COMMAND_CAPACITY,
>;

pub static CORE_MAILBOX: CoreMailbox = LightingMailbox::new();
#[cfg(feature = "rynk")]
static RYNK_MAILBOX: RynkLightingMailbox = RynkLightingMailbox::new();

static HIT_QUEUE: HitQueue<REACTIVE_HITS> = HitQueue::new();

/// Queue a key press for the Reactive effect and request a fresh render.
pub fn record_key_hit(led_idx: u8, timer_ms: u32) {
    if HIT_QUEUE.record(led_idx, timer_ms) {
        CORE_MAILBOX.snapshot_changed();
    }
}

/// Frame sink for the two IS31FL3731 chips on the shared I2C bus. Logical
/// slot i is `LED_TABLE[i]` by construction: the keyboard.toml emitters are
/// declared in `LED_TABLE` order.
pub struct Is31Output<'i> {
    i2c: &'i SharedI2c,
    rgb: Rgb,
}

impl<'i> Is31Output<'i> {
    fn new(i2c: &'i SharedI2c) -> Self {
        Self {
            i2c,
            rgb: Rgb::new(),
        }
    }

    async fn flush(&mut self) -> Result<(), ()> {
        let mut bus = self.i2c.lock().await;
        self.rgb.flush(&mut bus)
    }
}

impl LightingOutput<LogicalFrame<Rgb8, LED_COUNT>> for Is31Output<'_> {
    type Error = ();

    async fn initialize(&mut self) -> Result<(), Self::Error> {
        let mut bus = self.i2c.lock().await;
        is31fl3731::init_chip(&mut bus, ADDR_LEFT).await?;
        is31fl3731::init_chip(&mut bus, ADDR_RIGHT).await
    }

    async fn present(&mut self, frame: &LogicalFrame<Rgb8, LED_COUNT>) -> Result<(), Self::Error> {
        self.rgb.paint_rgb8(frame.as_array());
        self.flush().await
    }

    async fn suspend(&mut self) -> Result<(), Self::Error> {
        self.rgb.set_all(0, 0, 0);
        self.flush().await
    }

    async fn resume(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn retry_after(&self, _operation: OutputOperation, _error: &Self::Error) -> Option<NonZeroU32> {
        NonZeroU32::new(50)
    }
}

fn engine() -> Engine {
    // 0x30 initial brightness and speed 128 were tuned on hardware; boot on
    // the Afterburn palette like the old firmware.
    let palettefx = PaletteFxSource::new(
        VoyagerLayout,
        &HIT_QUEUE,
        PaletteFxConfig {
            initial_val: 0x30,
            initial_palette: id::AFTERBURN,
            ..PaletteFxConfig::default()
        },
    );
    Engine::new(
        crate::LIGHTING_BACKGROUND,
        crate::LIGHTING_LAYER_SCENES,
        palettefx,
        EmptySource,
    )
    .with_controls(crate::LIGHTING_CONTROLS)
}

pub fn init<'keymap, 'data, 'i>(
    keymap: &'keymap KeyMap<'data>,
    #[cfg(feature = "rynk")] persisted_scenes: &[LightingSceneCell],
    #[cfg(feature = "rynk")] persisted_policy: Option<LightingLayerPolicy>,
    i2c: &'i SharedI2c,
) -> LightingProcessor<
    'static,
    KeymapLightingState<'keymap, 'data>,
    Engine,
    Is31Output<'i>,
    COMMAND_CAPACITY,
> {
    let provider =
        KeymapLightingState::new(keymap).expect("Voyager layer count fits lighting state");
    #[cfg_attr(not(feature = "rynk"), allow(unused_mut))]
    let mut engine = engine();
    #[cfg(feature = "rynk")]
    install_lighting_scenes(
        &mut engine,
        &crate::LIGHTING_TOPOLOGY,
        persisted_scenes,
        persisted_policy,
    );
    let service = LightingService::new(provider, engine, LogicalFrame::new(Rgb8::BLACK));
    LightingProcessor::new(service, Is31Output::new(i2c), &CORE_MAILBOX)
}

#[cfg(feature = "rynk")]
pub fn rynk_adapter()
-> StandardRynkLightingAdapter<'static, OVERLAY_CAPACITY, COMMAND_CAPACITY, SCENE_CAPACITY> {
    StandardRynkLightingAdapter::new(&RYNK_MAILBOX, &CORE_MAILBOX, crate::LIGHTING_TOPOLOGY)
}

#[cfg(feature = "rynk")]
pub const fn rynk_controller() -> RynkLightingController<'static> {
    RynkLightingController::new(
        &RYNK_MAILBOX,
        RynkLightingDescriptor {
            topology_revision: crate::LIGHTING_TOPOLOGY_REVISION,
            topology: crate::LIGHTING_TOPOLOGY,
            routing: crate::LIGHTING_ROUTING,
        },
        OVERLAY_CAPACITY as u16,
    )
    .with_scene_capacity(SCENE_CAPACITY as u16)
    .with_conditional_scenes(&crate::LIGHTING_CONDITIONAL_SCENE_CELLS)
    .with_controls(crate::LIGHTING_CONTROLS)
}
