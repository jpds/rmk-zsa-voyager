use rmk::types::action::KeyAction;
use rmk::types::modifier::ModifierCombination;
use rmk::{a, k, kbctrl, layer, lt, mt, shifted};

pub(crate) const ROW: usize = 6;
pub(crate) const COL: usize = 7;
pub(crate) const NUM_LAYER: usize = 3;

/// Left-half keymap for the ZSA Voyager.
///   - Cols 0-6 map to PA0, PA1, PA2, PA3, PA6, PA7, PB0
///   - Col 0 is only used for thumb keys on row 5
///   - The bottom-right corner of the left hand (physical B on layer 0,
///     } on layer 1) lives at matrix (4, 4), not (3, 6)
///   - Thumb keys are on row 5 cols 0 and 1
/// Right-half cols are unwired until step 4; anything that would be on
/// the right half is simply absent.
#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        // Layer 0 — base
        //   Esc 1 2 3 4 5         | 6 7 8 9 0 -
        //   CW  Q W E R T         | Y U I O P \
        //   SBS A S D F G         | H J K L ; SQ
        //   GUI Z X C V B         | N M , . / R_Ctrl
        //                    Ent Tab | BSPC Spc
        layer!([
            [ a!(No),         k!(Escape),                                  k!(Kc1), k!(Kc2), k!(Kc3), k!(Kc4), k!(Kc5) ],
            [ a!(No),         kbctrl!(CapsWordToggle),                     k!(Q),   k!(W),   k!(E),   k!(R),   k!(T)   ],
            [ a!(No),         mt!(Backspace, ModifierCombination::LSHIFT), k!(A),   k!(S),   k!(D),   k!(F),   k!(G)   ],
            [ a!(No),         k!(LGui),                                    mt!(Z, ModifierCombination::LALT), k!(X), k!(C), k!(V), a!(No) ],
            [ a!(No),         a!(No),                                      a!(No),  a!(No),  k!(B),   a!(No),  a!(No)  ],
            [ lt!(1, Enter),  mt!(Tab, ModifierCombination::LCTRL),        a!(No),  a!(No),  a!(No),  a!(No),  a!(No)  ]
        ]),
        // Layer 1 — numbers + symbols (held via Enter thumb)
        //   Esc F1 F2 F3 F4 F5    | F6 F7 F8 F9 F10 F11
        //   `   !  @  #  $  %     | 7 8 9 -  /  F12
        //   __  ^  &  *  (  )     | 4 5 6 +  *  BSPC
        //   __  __ [  ]  {  }     | 1 2 3 .  =  Ent
        //                    __ __ | __ 0
        layer!([
            [ a!(No),          k!(Escape),        k!(F1),         k!(F2),         k!(F3),         k!(F4),          k!(F5)  ],
            [ a!(No),          k!(Grave),         shifted!(Kc1),  shifted!(Kc2),  shifted!(Kc3),  shifted!(Kc4),   shifted!(Kc5) ],
            [ a!(No),          a!(Transparent),   shifted!(Kc6),  shifted!(Kc7),  shifted!(Kc8),  shifted!(Kc9),   shifted!(Kc0) ],
            [ a!(No),          a!(Transparent),   a!(Transparent),k!(LeftBracket),k!(RightBracket),shifted!(LeftBracket), a!(No) ],
            [ a!(No),          a!(No),            a!(No),         a!(No),         shifted!(RightBracket), a!(No),  a!(No)  ],
            [ a!(Transparent), a!(Transparent),   a!(No),         a!(No),         a!(No),         a!(No),          a!(No)  ]
        ]),
        // Layer 2 — media + navigation (reachable via Space thumb once
        // right half is wired; currently unreachable from left alone)
        //   __  __  __  __   __   __   | __   __   __   __   __   Boot
        //   __  __  VOL- VOL+ Mute __  | PgUp Home Up   End  __   __
        //   __  PrT NxT  StT  Ply  __  | PgDn Lft  Dn   Rght __   __
        //   __  __  __   __   __   __  | __   C-S-Tab C-Tab __ __ __
        //                         __ __ | __ __
        layer!([
            [ a!(No),          a!(No),           a!(No),             a!(No),             a!(No),             a!(No),            a!(No) ],
            [ a!(No),          a!(Transparent),     a!(Transparent),     k!(AudioVolDown),   k!(AudioVolUp),     k!(AudioMute),      a!(Transparent) ],
            [ a!(No),          a!(Transparent),     k!(MediaPrevTrack),  k!(MediaNextTrack), k!(MediaStop),      k!(MediaPlayPause), a!(Transparent) ],
            [ a!(Transparent), a!(Transparent), a!(Transparent),    a!(Transparent),    a!(Transparent),    a!(Transparent),   a!(No) ],
            [ a!(No),          a!(No),          a!(No),             a!(No),             a!(Transparent),    a!(No),            a!(No) ],
            [ a!(Transparent), a!(Transparent), a!(No),             a!(No),             a!(No),             a!(No),            a!(No) ]
        ])
    ]
}
