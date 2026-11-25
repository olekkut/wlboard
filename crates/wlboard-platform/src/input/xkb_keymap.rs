//! Dynamic XKB keymap string generator from any `LayoutDefinition`
//! for Wayland `zwp_virtual_keyboard_v1` (Hyprland, Sway, wlroots compositors).

use std::collections::HashMap;
use std::fmt::Write as _;

use wlboard_core::{KeyCode, LayoutDefinition};

/// Convert a single UTF-8 display string into an XKB keysym name (`UXXXX` or standard name).
fn label_to_xkb_sym(label: &str) -> String {
    // Handle multi-character Arabic lam-alef ligatures cleanly via standard Unicode isolated ligature keysyms.
    match label {
        "لا" => return "UFEFB".to_owned(),
        "لأ" => return "UFEF7".to_owned(),
        "لإ" => return "UFEF9".to_owned(),
        "لآ" => return "UFEF5".to_owned(),
        _ => {}
    }

    let Some(ch) = label.chars().next() else {
        return "NoSymbol".to_owned();
    };
    let code = ch as u32;
    if code < 0x20 {
        return "NoSymbol".to_owned();
    }
    format!("U{code:04X}")
}

/// Generate a complete, valid, standalone XKB v1 keymap for the given `LayoutDefinition`.
///
/// The resulting string is NUL-free and ready to be uploaded with a trailing `\0` byte
/// over `zwp_virtual_keyboard_v1::keymap`.
#[must_use]
pub fn build_xkb_keymap(layout: &LayoutDefinition) -> String {
    let mut out = String::with_capacity(8192);
    out.push_str(
        r#"xkb_keymap {
xkb_keycodes "wlboard_evdev" {
    minimum = 8;
    maximum = 255;
    <ESC>  = 9;
    <AE01> = 10; <AE02> = 11; <AE03> = 12; <AE04> = 13; <AE05> = 14; <AE06> = 15;
    <AE07> = 16; <AE08> = 17; <AE09> = 18; <AE10> = 19; <AE11> = 20; <AE12> = 21;
    <BKSP> = 22;
    <TAB>  = 23;
    <AD01> = 24; <AD02> = 25; <AD03> = 26; <AD04> = 27; <AD05> = 28; <AD06> = 29;
    <AD07> = 30; <AD08> = 31; <AD09> = 32; <AD10> = 33; <AD11> = 34; <AD12> = 35;
    <RTRN> = 36;
    <LCTL> = 37;
    <AC01> = 38; <AC02> = 39; <AC03> = 40; <AC04> = 41; <AC05> = 42; <AC06> = 43;
    <AC07> = 44; <AC08> = 45; <AC09> = 46; <AC10> = 47; <AC11> = 48;
    <TLDE> = 49;
    <LFSH> = 50;
    <BKSL> = 51;
    <AB01> = 52; <AB02> = 53; <AB03> = 54; <AB04> = 55; <AB05> = 56; <AB06> = 57;
    <AB07> = 58; <AB08> = 59; <AB09> = 60; <AB10> = 61;
    <RTSH> = 62;
    <LALT> = 64;
    <SPCE> = 65;
    <CAPS> = 66;
    <FK01> = 67; <FK02> = 68; <FK03> = 69; <FK04> = 70; <FK05> = 71; <FK06> = 72;
    <FK07> = 73; <FK08> = 74; <FK09> = 75; <FK10> = 76; <FK11> = 95; <FK12> = 96;
    <RCTL> = 105;
    <PRSC> = 107;
    <RALT> = 108;
    <HOME> = 110;
    <UP>   = 111;
    <PGUP> = 112;
    <LEFT> = 113;
    <RGHT> = 114;
    <END>  = 115;
    <DOWN> = 116;
    <PGDN> = 117;
    <INS>  = 118;
    <DELE> = 119;
    <MUTE> = 121;
    <VOL-> = 122;
    <VOL+> = 123;
    <LWIN> = 133;
    <I171> = 171;
    <I172> = 172;
    <I173> = 173;
    <I232> = 232;
    <I233> = 233;
};

xkb_types "wlboard_types" {
    virtual_modifiers LevelThree, Alt, Super;
    type "ONE_LEVEL" {
        modifiers = none;
        level_name[Level1] = "Any";
    };
    type "TWO_LEVEL" {
        modifiers = Shift;
        map[Shift] = Level2;
        level_name[Level1] = "Base";
        level_name[Level2] = "Shift";
    };
    type "ALPHABETIC" {
        modifiers = Shift+Lock;
        map[Shift] = Level2;
        map[Lock] = Level2;
        level_name[Level1] = "Base";
        level_name[Level2] = "Caps";
    };
    type "FOUR_LEVEL_ALPHABETIC" {
        modifiers = Shift+Lock+LevelThree;
        map[Shift] = Level2;
        map[Lock] = Level2;
        map[LevelThree] = Level3;
        map[Shift+LevelThree] = Level4;
        map[Lock+LevelThree] = Level4;
        level_name[Level1] = "Base";
        level_name[Level2] = "Shift";
        level_name[Level3] = "AltGr";
        level_name[Level4] = "AltGr+Shift";
    };
};

xkb_compatibility "wlboard_compat" {
    virtual_modifiers LevelThree, Alt, Super;
    interpret Shift_L { action = SetMods(modifiers=Shift); };
    interpret Shift_R { action = SetMods(modifiers=Shift); };
    interpret Control_L { action = SetMods(modifiers=Control); };
    interpret Control_R { action = SetMods(modifiers=Control); };
    interpret Caps_Lock { action = LockMods(modifiers=Lock); };
    interpret Alt_L {
        virtualModifier = Alt;
        action = SetMods(modifiers=modMapMods);
    };
    interpret Super_L {
        virtualModifier = Super;
        action = SetMods(modifiers=modMapMods);
    };
    interpret ISO_Level3_Shift {
        virtualModifier = LevelThree;
        action = SetMods(modifiers=modMapMods);
    };
};

xkb_symbols "wlboard_symbols" {
"#,
    );

    let _ = writeln!(
        out,
        "    name[group1] = \"{}\";",
        layout.display_name.replace('"', "")
    );

    // Grave key
    let _ = writeln!(
        out,
        "    key <TLDE> {{ type = \"TWO_LEVEL\", [ {}, {} ] }};",
        label_to_xkb_sym(&layout.grave.normal),
        label_to_xkb_sym(&layout.grave.shift)
    );

    // Digits 1..0 - =
    let digit_tags = [
        ("<AE01>", "1"),
        ("<AE02>", "2"),
        ("<AE03>", "3"),
        ("<AE04>", "4"),
        ("<AE05>", "5"),
        ("<AE06>", "6"),
        ("<AE07>", "7"),
        ("<AE08>", "8"),
        ("<AE09>", "9"),
        ("<AE10>", "0"),
        ("<AE11>", "-"),
        ("<AE12>", "="),
    ];
    for (idx, (tag, norm)) in digit_tags.iter().enumerate() {
        let sh = layout
            .digits_shift
            .get(idx)
            .map(String::as_str)
            .unwrap_or(norm);
        let _ = writeln!(
            out,
            "    key {tag} {{ type = \"TWO_LEVEL\", [ {}, {} ] }};",
            label_to_xkb_sym(norm),
            label_to_xkb_sym(sh)
        );
    }

    // Character keycode -> XKB tag lookup
    let tag_map = keycode_to_xkb_tag();
    for key in layout
        .top_row
        .iter()
        .chain(layout.home_row.iter())
        .chain(layout.bottom_row.iter())
    {
        let Some(kc) = KeyCode::from_name(&key.code) else {
            continue;
        };
        let Some(&tag) = tag_map.get(&kc) else {
            continue;
        };
        let altgr_override = layout.altgr.get(&key.code);
        let l1 = label_to_xkb_sym(&key.normal);
        let l2 = label_to_xkb_sym(&key.shift);
        let l3 = key
            .altgr
            .as_deref()
            .or_else(|| altgr_override.map(|p| p.normal.as_str()))
            .map(label_to_xkb_sym);
        let l4 = key
            .altgr_shift
            .as_deref()
            .or_else(|| altgr_override.map(|p| p.shift.as_str()))
            .map(label_to_xkb_sym);

        if let Some(a3) = l3 {
            let a4 = l4.unwrap_or_else(|| a3.clone());
            let _ = writeln!(
                out,
                "    key {tag} {{ type = \"FOUR_LEVEL_ALPHABETIC\", [ {l1}, {l2}, {a3}, {a4} ] }};"
            );
        } else {
            let is_alpha = key.normal.chars().next().is_some_and(|c| c.is_alphabetic());
            let ktype = if is_alpha { "ALPHABETIC" } else { "TWO_LEVEL" };
            let _ = writeln!(
                out,
                "    key {tag} {{ type = \"{ktype}\", [ {l1}, {l2} ] }};"
            );
        }
    }

    // Action, modifier, navigation, function, and media keys
    out.push_str(
        r#"    key <ESC>  { [ Escape ] };
    key <BKSP> { [ BackSpace ] };
    key <TAB>  { [ Tab ] };
    key <RTRN> { [ Return ] };
    key <SPCE> { [ space ] };
    key <LFSH> { [ Shift_L ] };
    key <RTSH> { [ Shift_R ] };
    key <LCTL> { [ Control_L ] };
    key <RCTL> { [ Control_R ] };
    key <CAPS> { [ Caps_Lock ] };
    key <LALT> { [ Alt_L ] };
    key <RALT> { [ ISO_Level3_Shift ] };
    key <LWIN> { [ Super_L ] };
    key <PRSC> { [ Print ] };
    key <HOME> { [ Home ] };
    key <UP>   { [ Up ] };
    key <PGUP> { [ Prior ] };
    key <LEFT> { [ Left ] };
    key <RGHT> { [ Right ] };
    key <END>  { [ End ] };
    key <DOWN> { [ Down ] };
    key <PGDN> { [ Next ] };
    key <INS>  { [ Insert ] };
    key <DELE> { [ Delete ] };
    key <FK01> { [ F1 ] };
    key <FK02> { [ F2 ] };
    key <FK03> { [ F3 ] };
    key <FK04> { [ F4 ] };
    key <FK05> { [ F5 ] };
    key <FK06> { [ F6 ] };
    key <FK07> { [ F7 ] };
    key <FK08> { [ F8 ] };
    key <FK09> { [ F9 ] };
    key <FK10> { [ F10 ] };
    key <FK11> { [ F11 ] };
    key <FK12> { [ F12 ] };
    key <MUTE> { [ XF86AudioMute ] };
    key <VOL-> { [ XF86AudioLowerVolume ] };
    key <VOL+> { [ XF86AudioRaiseVolume ] };
    key <I171> { [ XF86AudioNext ] };
    key <I172> { [ XF86AudioPlay ] };
    key <I173> { [ XF86AudioPrev ] };
    key <I232> { [ XF86MonBrightnessDown ] };
    key <I233> { [ XF86MonBrightnessUp ] };

    modifier_map Shift   { <LFSH>, <RTSH> };
    modifier_map Lock    { <CAPS> };
    modifier_map Control { <LCTL>, <RCTL> };
    modifier_map Mod1    { <LALT> };
    modifier_map Mod4    { <LWIN> };
    modifier_map Mod5    { <RALT> };
};
};
"#,
    );

    out
}

fn keycode_to_xkb_tag() -> HashMap<KeyCode, &'static str> {
    HashMap::from([
        (KeyCode::Q, "<AD01>"),
        (KeyCode::W, "<AD02>"),
        (KeyCode::E, "<AD03>"),
        (KeyCode::R, "<AD04>"),
        (KeyCode::T, "<AD05>"),
        (KeyCode::Y, "<AD06>"),
        (KeyCode::U, "<AD07>"),
        (KeyCode::I, "<AD08>"),
        (KeyCode::O, "<AD09>"),
        (KeyCode::P, "<AD10>"),
        (KeyCode::LEFTBRACE, "<AD11>"),
        (KeyCode::RIGHTBRACE, "<AD12>"),
        (KeyCode::BACKSLASH, "<BKSL>"),
        (KeyCode::A, "<AC01>"),
        (KeyCode::S, "<AC02>"),
        (KeyCode::D, "<AC03>"),
        (KeyCode::F, "<AC04>"),
        (KeyCode::G, "<AC05>"),
        (KeyCode::H, "<AC06>"),
        (KeyCode::J, "<AC07>"),
        (KeyCode::K, "<AC08>"),
        (KeyCode::L, "<AC09>"),
        (KeyCode::SEMICOLON, "<AC10>"),
        (KeyCode::APOSTROPHE, "<AC11>"),
        (KeyCode::Z, "<AB01>"),
        (KeyCode::X, "<AB02>"),
        (KeyCode::C, "<AB03>"),
        (KeyCode::V, "<AB04>"),
        (KeyCode::B, "<AB05>"),
        (KeyCode::N, "<AB06>"),
        (KeyCode::M, "<AB07>"),
        (KeyCode::COMMA, "<AB08>"),
        (KeyCode::DOT, "<AB09>"),
        (KeyCode::SLASH, "<AB10>"),
    ])
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use wlboard_core::{BUILTIN_LAYOUT_CODES, LayoutRegistry};

    use super::*;

    #[test]
    fn generated_xkb_keymaps_compile_with_xkbcomp() {
        let reg = LayoutRegistry::new();
        let has_xkbcomp = Command::new("xkbcomp").arg("-version").output().is_ok();
        for code in BUILTIN_LAYOUT_CODES {
            let keymap = build_xkb_keymap(reg.get(code));
            assert!(keymap.contains("xkb_keymap"));
            assert!(!keymap.contains('\0'));

            if has_xkbcomp {
                let out_path = std::env::temp_dir().join(format!(
                    "wlboard_xkb_test_{}_{code}.xkm",
                    std::process::id()
                ));
                let mut child = Command::new("xkbcomp")
                    .args(["-w", "0", "-"])
                    .arg(&out_path)
                    .stdin(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("spawn xkbcomp");
                child
                    .stdin
                    .as_mut()
                    .expect("stdin")
                    .write_all(keymap.as_bytes())
                    .expect("write keymap");
                let out = child.wait_with_output().expect("wait xkbcomp");
                let _ = std::fs::remove_file(&out_path);
                assert!(
                    out.status.success(),
                    "xkbcomp failed for {code}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }
}
