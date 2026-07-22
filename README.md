# rmk-zsa-voyager

RMK firmware for the [ZSA Voyager](https://www.zsa.io/voyager) split keyboard.

The Voyager ships with a GD32F303CB microcontroller (STM32F303CB-compatible). This firmware
targets that MCU directly using [RMK](https://rmk.rs) and the
[Embassy](https://embassy.dev) async embedded Rust ecosystem, replacing ZSA's
stock QMK-based firmware.

## Features

- **Topology-aware lighting** - the per-key RGB matrix is driven by RMK's
  composable lighting engine ([colonelpanic8/rmk](https://github.com/colonelpanic8/rmk)):
  a declarative `[lighting]` topology in `keyboard.toml` (52 emitters across the
  two IS31FL3731 chips), per-layer scenes composited over an animated
  extension band, and a master output mode
- **Animated per-key RGB** - six switchable effects powered by [rmk-palettefx](https://github.com/jpds/rmk-palettefx):
  Gradient, Flow, Vortex, Sparkle, Ripple, and Reactive (key-press ripples),
  plugged into the lighting engine's extension band via palettefx's
  `rmk-lighting` adapter (reusable by any board with an `LedLayout`)
- **16 built-in colour palettes** - cycle with the RGB hue keys
- **Layer status LEDs** - 4-bit binary display of the active layer
- **Chordal hold** - unilateral-tap behaviour (mod-taps resolve instantly when both keys are on
  the same hand), matching the QMK PermissiveHold feel

## Keymap

Three compiled-in layers using the [default Voyager
layout](https://www.zsa.io/assets/voyager/default-layout.pdf).

Vial support was dropped in the move to the lighting-abstraction RMK fork.
Its successor, the Rynk protocol (host-side keymap and lighting control), is
wired up behind the `rynk` cargo feature, but the full Rynk dispatch surface
currently adds ~80 KB of code and does not fit the F303's 128 KB flash
(183 KB vs the 116 KB usable region); build with `--features rynk` to track
it.

## Building

Fetch the vendored RMK fork and rmk-palettefx (git submodules under
`dependencies/`):

```sh
git submodule update --init
```

Either use the Nix development shell environment provided in `flake.nix`:

```sh
nix develop
```

Or install the Rust toolchain and the Cortex-M4F target:

```sh
rustup target add thumbv7em-none-eabihf
```

Build a release binary:

```sh
cargo build --release
cargo objcopy --release -- -O binary rmk-zsa-voyager.bin
```

## Flashing

Put the Voyager into DFU mode (press the reset button on the top-side of the board,
or press the `Bootloader` key from Layer 2).

Flash with `dfu-util`:

```sh
dfu-util -d 3297:0791 -a 0 -s 0x08002000:leave -D rmk-zsa-voyager.bin
```

Or use [ZSA Keymapp](https://www.zsa.io/flash) to flash the `.bin` file via its graphical
interface.

Flashing is safe: the DFU bootloader lives in protected flash and cannot be overwritten by a
firmware image.

## Dependencies

Both RMK and rmk-palettefx are pinned as git submodules under `dependencies/`
and consumed as path dependencies, following the pattern used by
[glove80-rmk](https://github.com/colonelpanic8/glove80-rmk).

| Crate | Role |
|-------|------|
| [rmk (colonelpanic8 fork)](https://github.com/colonelpanic8/rmk) | Keyboard framework (key scanning, HID, storage, composable lighting engine, Rynk) |
| [embassy-stm32](https://embassy.dev) | Async HAL for STM32F303 |
| [rmk-palettefx](https://github.com/jpds/rmk-palettefx) | Palette-driven RGB animation effects + `LightingSource` adapter |

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your
option.

---

This project is not affiliated with or endorsed by ZSA Technology Labs, Inc.
