use rmk::config::Hand;
use rmk::types::action::KeyAction;
use rmk::types::modifier::ModifierCombination;
use rmk::{a, k, kbctrl, layer, light, lt, mt, shifted, wm};

/// Index into `behavior_config.morse.morses`.
pub const TD_ESC_EQL: u8 = 0;

pub(crate) const ROW: usize = 12;
pub(crate) const COL: usize = 7;
pub(crate) const NUM_LAYER: usize = 3;

/// Per-cell hand assignment for unilateral-tap (chordal hold):
/// a morse key held while another key on the same hand is pressed
/// resolves as tap immediately. Voyager matrix is cleanly split -
/// rows 0-4 are the left half alphas (direct GPIO), rows 6-10 are
/// the right half alphas (MCP23018). Row 5 / row 11 are the thumb
/// clusters and are left `Hand::Unknown` so thumb mods never get
/// instant-tapped by a same-hand alpha press.
#[rustfmt::skip]
pub const HAND_MAP: [[Hand; COL]; ROW] = [
    [Hand::Left; COL], [Hand::Left; COL],  [Hand::Left; COL],
    [Hand::Left; COL], [Hand::Left; COL],  [Hand::Unknown; COL],
    [Hand::Right; COL], [Hand::Right; COL], [Hand::Right; COL],
    [Hand::Right; COL], [Hand::Right; COL], [Hand::Unknown; COL],
];

/// Default Voyager keymap.
///
/// Left half rows 0-5:
///   - Cols 0-6 map to PA0, PA1, PA2, PA3, PA6, PA7, PB0
///   - Col 0 is only used for thumb keys on row 5
///   - The left-hand `B` / `}` lives at matrix (4, 4), not (3, 6)
///
/// Right half rows 6-11 arrive via the MCP23018 with axes swapped:
///   - logical row = 11 - (Port B bit)   (sense line)
///   - logical col =  6 - (Port A bit)   (strobe line)
///   - Right-hand `N` lives at matrix (10, 2), thumbs at (11, 5/6)
#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        // Layer 0 - BASE (QWERTY)
        //
        //                     LEFT HALF                                 RIGHT HALF
        //  row 0 :   ESC    1    2    3    4    5              6    7    8    9    0    -
        //  row 1 :   CAPW   Q    W    E    R    T              Y    U    I    O    P    \
        //  row 2 :   BKS*   A    S    D    F    G              H    J    K    L    ;    '*
        //             sft                                                                sft
        //  row 3 :   GUI    Z*   X    C    V    B              N    M    ,    .    /*   CTL
        //                   alt                                                     alt
        //  thumbs :                         [ENT]              [SPC]
        //                                    SYM↓ [TAB]  [BKS]  NAV↓
        //                                          CTL↓   SFT↓
        //
        //  *  = mod-tap (tap = key, hold = modifier shown below); ↓ = layer/mod on hold
        //  CAPW = CapsWord toggle
        //  BKS (row 2, outer pinky) = mt!(Backspace, LSHIFT)
        //  ENT = LT(SYM, Enter); TAB = mt!(Tab, LCTRL)
        //  BKS (right thumb) = mt!(Backspace, RSHIFT); SPC = LT(NAV, Space)
        //  The inner-index "drop" keys (B on left, N on right) are physically
        //  part of row 3 but wired to matrix (4, 4) / (10, 2).
        layer!([
            // Left half
            [ a!(No),         KeyAction::Morse(TD_ESC_EQL),                k!(Kc1), k!(Kc2), k!(Kc3), k!(Kc4), k!(Kc5) ],
            [ a!(No),         kbctrl!(CapsWordToggle),                     k!(Q),   k!(W),   k!(E),   k!(R),   k!(T)   ],
            [ a!(No),         mt!(Backspace, ModifierCombination::LSHIFT), k!(A),   k!(S),   k!(D),   k!(F),   k!(G)   ],
            [ a!(No),         k!(LGui),                                    mt!(Z, ModifierCombination::LALT), k!(X), k!(C), k!(V), a!(No) ],
            [ a!(No),         a!(No),                                      a!(No),  a!(No),  k!(B),   a!(No),  a!(No)  ],
            [ lt!(1, Enter),  mt!(Tab, ModifierCombination::LCTRL),        a!(No),  a!(No),  a!(No),  a!(No),  a!(No)  ],
            // Right half
            [ k!(Kc6),        k!(Kc7),   k!(Kc8),       k!(Kc9),   k!(Kc0),                              k!(Minus),                                a!(No) ],
            [ k!(Y),          k!(U),     k!(I),         k!(O),     k!(P),                                k!(Backslash),                            a!(No) ],
            [ k!(H),          k!(J),     k!(K),         k!(L),     k!(Semicolon),                        mt!(Quote, ModifierCombination::RSHIFT),  a!(No) ],
            [ a!(No),         k!(M),     k!(Comma),     k!(Dot),   mt!(Slash, ModifierCombination::RALT), k!(RCtrl),                               a!(No) ],
            [ a!(No),         a!(No),    k!(N),         a!(No),    a!(No),                               a!(No),                                   a!(No) ],
            [ a!(No),         a!(No),    a!(No),        a!(No),    a!(No),                               mt!(Backspace, ModifierCombination::RSHIFT), lt!(2, Space) ]
        ]),
        // Layer 1 - SYM (held via left inner thumb LT(1, Enter))
        //
        //                     LEFT HALF                                 RIGHT HALF
        //  row 0 :   ESC    F1   F2   F3   F4   F5             F6   F7   F8   F9   F10  F11
        //  row 1 :   `      !    @    #    $    %              7    8    9    -    /    F12
        //  row 2 :   ·      ^    &    *    (    )              4    5    6    +    *    BKS
        //  row 3 :   ·      ·    [    ]    {    }              1    2    3    .    =    ENT
        //  thumbs :                         [ · ]              [ 0 ]
        //                                         [ · ]  [ · ]
        //
        //  · = a!(Transparent) or a!(No) (falls through to BASE)
        //  Numpad layout on the right half (7/8/9, 4/5/6, 1/2/3, 0 on thumb).
        //  Brackets / braces on the left half; shifted number-row symbols
        //  (!, @, #, $, %, ^, &, *, (, )) on rows 1 and 2.
        layer!([
            // Left half
            [ a!(No),          k!(Escape),        k!(F1),         k!(F2),         k!(F3),         k!(F4),          k!(F5)  ],
            [ a!(No),          k!(Grave),         shifted!(Kc1),  shifted!(Kc2),  shifted!(Kc3),  shifted!(Kc4),   shifted!(Kc5) ],
            [ a!(No),          a!(Transparent),   shifted!(Kc6),  shifted!(Kc7),  shifted!(Kc8),  shifted!(Kc9),   shifted!(Kc0) ],
            [ a!(No),          a!(Transparent),   a!(Transparent),k!(LeftBracket),k!(RightBracket),shifted!(LeftBracket), a!(No) ],
            [ a!(No),          a!(No),            a!(No),         a!(No),         shifted!(RightBracket), a!(No),  a!(No)  ],
            [ a!(Transparent), a!(Transparent),   a!(No),         a!(No),         a!(No),         a!(No),          a!(No)  ],
            // Right half
            [ k!(F6),          k!(F7),   k!(F8),        k!(F9),    k!(F10),                              k!(F11),                                  a!(No) ],
            [ k!(Kc7),         k!(Kc8),  k!(Kc9),       k!(Minus), k!(Slash),                            k!(F12),                                  a!(No) ],
            [ k!(Kc4),         k!(Kc5), k!(Kc6),        shifted!(Equal), shifted!(Kc8),                  k!(Backspace),                            a!(No) ],
            [ a!(No),          k!(Kc2), k!(Kc3),        k!(Dot),   k!(Equal),                           k!(Enter),                                a!(No) ],
            [ a!(No),          a!(No),  k!(Kc1),        a!(No),    a!(No),                              a!(No),                                   a!(No) ],
            [ a!(No),          a!(No),  a!(No),         a!(No),    a!(No),                              a!(Transparent),                          k!(Kc0) ]
        ]),
        // Layer 2 - NAV (held via right outer thumb LT(2, Space))
        //
        //                     LEFT HALF                                 RIGHT HALF
        //  row 0 :   ·      ·    MODE ·    VAD  VAI            ·    ·    ·    ·    ·    BOOT
        //  row 1 :   HUI    ·    V-   V+   MUT  ·              PgU  HOM  UP   END  ·    ·
        //  row 2 :   ·      Prv  Nxt  Stp  Play ·              PgD  ←    ↓    →    ·    ·
        //  row 3 :   ·      ·    ·    ·    ·    ·              ·    WinR Win  ·    ·    ·
        //  thumbs :                         [ · ]              [ · ]
        //                                         [ · ]  [ · ]
        //
        //  · = a!(Transparent) or a!(No) (falls through to BASE)
        //  MODE = RgbModeForward; HUI = RgbHueIncr
        //  VAI/VAD = RGB value up/down; V-/V+ = system volume down/up
        //  MUT = AudioMute; Prv/Nxt/Stp/Play = media controls
        //  PgU/PgD, HOM, UP, END, ←/↓/→ = navigation cluster
        //  WinR = Ctrl+Shift+Tab (previous window);
        //  Win  = Ctrl+Tab         (next window)
        //  BOOT = enter bootloader (DFU)
        layer!([
            // Left half
            [ a!(No),          a!(No),              a!(No),              light!(RgbModeForward), a!(No),          light!(RgbVad),     light!(RgbVai)   ],
            [ a!(No),          light!(RgbHui),      a!(Transparent),     k!(AudioVolDown),       k!(AudioVolUp),  k!(AudioMute),      a!(Transparent)  ],
            [ a!(No),          a!(Transparent),     k!(MediaPrevTrack),  k!(MediaNextTrack),     k!(MediaStop),   k!(MediaPlayPause), a!(Transparent)  ],
            [ a!(Transparent), a!(Transparent), a!(Transparent),    a!(Transparent),    a!(Transparent),    a!(Transparent),   a!(No) ],
            [ a!(No),          a!(No),          a!(No),             a!(No),             a!(Transparent),    a!(No),            a!(No) ],
            [ a!(Transparent), a!(Transparent), a!(No),             a!(No),             a!(No),             a!(No),            a!(No) ],
            // Right half
            [ a!(Transparent), a!(Transparent), a!(Transparent),   a!(Transparent),   a!(Transparent),                        kbctrl!(Bootloader),                      a!(No) ],
            [ k!(PageUp),      k!(Home),        k!(Up),            k!(End),           a!(Transparent),                        a!(Transparent),                          a!(No) ],
            [ k!(PageDown),    k!(Left),        k!(Down),          k!(Right),         a!(Transparent),                        a!(Transparent),                          a!(No) ],
            [ a!(No),          wm!(Tab, ModifierCombination::new_from(false, false, false, true, true)), wm!(Tab, ModifierCombination::LCTRL), a!(Transparent), a!(Transparent), a!(Transparent), a!(No) ],
            [ a!(No),          a!(No),          a!(Transparent),   a!(No),            a!(No),                                 a!(No),                                   a!(No) ],
            [ a!(No),          a!(No),          a!(No),            a!(No),            a!(No),                                 a!(Transparent),                          a!(Transparent) ]
        ])
    ]
}
