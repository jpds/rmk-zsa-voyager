# rmk-zsa-voyager

RMK firmware for the [ZSA Voyager](https://www.zsa.io/voyager) split keyboard.

The Voyager ships with a GD32F303CB microcontroller (STM32F303CB-compatible). This firmware
targets that MCU directly using [RMK](https://rmk.rs) and the
[Embassy](https://embassy.dev) async embedded Rust ecosystem, replacing ZSA's
stock QMK-based firmware.

## Features

- **Live keymap editing** via [Vial](https://get.vial.today/) - remap keys without reflashing
- **Animated per-key RGB** - six switchable effects powered by [rmk-palettefx](https://github.com/jpds/rmk-palettefx):
  Gradient, Flow, Vortex, Sparkle, Ripple, and Reactive (key-press ripples)
- **16 built-in colour palettes** - cycle with the RGB hue keys
- **Layer status LEDs** - 4-bit binary display of the active layer
- **Chordal hold** - unilateral-tap behaviour (mod-taps resolve instantly when both keys are on
  the same hand), matching the QMK PermissiveHold feel

## Keymap

Three compiled-in layers using the [default Voyager
layout](https://www.zsa.io/assets/voyager/default-layout.pdf); all are
live-editable via Vial.

## Building

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
dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D rmk-zsa-voyager.bin
```

Or use [ZSA Keymapp](https://www.zsa.io/flash) to flash the `.bin` file via its graphical
interface.

Flashing is safe: the DFU bootloader lives in protected flash and cannot be overwritten by a
firmware image.

## Dependencies

| Crate | Role |
|-------|------|
| [rmk](https://github.com/HaoboGu/rmk) | Keyboard framework (key scanning, HID, Vial, storage) |
| [embassy-stm32](https://embassy.dev) | Async HAL for STM32F303 |
| [rmk-palettefx](https://github.com/jpds/rmk-palettefx) | Palette-driven RGB animation effects |

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your
option.

---

This project is not affiliated with or endorsed by ZSA Technology Labs, Inc.
