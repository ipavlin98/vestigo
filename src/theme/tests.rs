use super::*;
use gpui::Rgba;

fn color(hex: u32) -> Hsla {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

fn load(files: &[(&str, &str)]) -> Catalog {
    Catalog::load(files.iter().map(|(id, text)| ((*id).to_owned(), (*text).to_owned())).collect())
}

fn named(catalog: &Catalog, id: &str) -> Theme {
    catalog.resolve(&ThemeChoice::Named(id.to_owned()), Appearance::Light, &ThemeEdits::default())
}

#[test]
fn built_in_themes_keep_the_original_palette() {
    let catalog = load(&[]);
    assert!(catalog.failures().is_empty());
    let dark = named(&catalog, "dark");
    assert_eq!(dark.launcher.background, color(0x000000));
    assert_eq!(dark.results.selected, color(0x1c1c1c));
    assert_eq!(dark.results.matched, color(0xffffff));
    assert_eq!(dark.keys.frame, color(0x282828));
    assert_eq!(dark.toggle.knob, color(0x000000));
    let light = named(&catalog, "light");
    assert_eq!(light.app.background, color(0xffffff));
    assert_eq!(light.nav.text, color(0x666666));
    assert_eq!(light.input.selection, color(0xd4d4d4));
    assert_eq!(light.radius, Radius { large: 6.0, small: 4.0, toggle: 12.0 });
    assert!(light.windows.dialog.shadow.is_empty());
    assert_eq!(light.windows.app.shadow.len(), 2);
    assert_eq!(light.font, "Segoe UI");
}

#[test]
fn elements_follow_the_roles_a_theme_changes() {
    let catalog = load(&[("pink", r##"{ "base": "dark", "palette": { "rose": "#ff0080", "accent": "rose" } }"##)]);
    let theme = named(&catalog, "pink");
    assert_eq!(theme.results.matched, color(0xff0080));
    assert_eq!(theme.input.caret, color(0xff0080));
    assert_eq!(theme.results.name, color(0xededed));
}

#[test]
fn an_element_can_point_at_any_role_or_color() {
    let catalog = load(&[(
        "custom",
        r##"{ "base": "light", "palette": { "brand": "#336699" }, "toggle": { "on": "brand", "knob": "#abc" } }"##,
    )]);
    let theme = named(&catalog, "custom");
    assert_eq!(theme.toggle.on, color(0x336699));
    assert_eq!(theme.toggle.knob, color(0xaabbcc));
    assert_eq!(theme.toggle.off, color(0xd4d4d4));
}

#[test]
fn corners_take_one_radius_or_each_corner_apart() {
    let catalog = load(&[(
        "round",
        r#"{ "base": "dark", "windows": { "launcher": { "corners": 12 }, "dialog": { "corners": { "bottom_left": 8 } } } }"#,
    )]);
    let windows = named(&catalog, "round").windows;
    assert_eq!(windows.launcher.corners.all(), [12.0; 4]);
    assert_eq!(windows.dialog.corners.all(), [0.0, 0.0, 0.0, 8.0]);
    assert_eq!(windows.explorer.corners.all(), [0.0; 4]);
    assert_eq!(windows.dialog.corners.flipped().all(), [8.0, 0.0, 0.0, 0.0], "a bar above the dialog rounds its top");
}

#[test]
fn each_window_casts_its_own_shadow() {
    let catalog = load(&[(
        "lit",
        r##"{ "base": "dark", "windows": { "launcher": { "shadow": [] }, "app": { "shadow": [{ "color": "#ff0080", "x": 2, "blur": 4 }] } } }"##,
    )]);
    let windows = named(&catalog, "lit").windows;
    assert!(windows.launcher.shadow.is_empty());
    assert_eq!(windows.explorer.shadow.len(), 2);
    let glow = ShadowLayer { color: color(0xff0080), opacity: 1.0, x: 2.0, y: 0.0, blur: 4.0, spread: 0.0 };
    assert_eq!(windows.app.shadow, [glow]);
}

#[test]
fn the_shipped_examples_load() {
    let catalog = load(&[]);
    assert!(catalog.failures().is_empty(), "{:?}", catalog.failures());
    let mocha = named(&catalog, "catppuccin-mocha");
    assert_eq!(mocha.launcher.background, color(0x1e1e2e));
    assert_eq!(mocha.results.matched, color(0xcba6f7));
    assert_eq!(mocha.windows.dialog.corners.all(), [0.0, 0.0, 12.0, 12.0]);
    assert_eq!(mocha.windows.explorer.corners.all(), [12.0, 0.0, 0.0, 0.0]);
    let sand = named(&catalog, "soft-sand");
    assert_eq!(sand.launcher.background, color(0xf6f3ec));
    assert_eq!(sand.toggle.on, color(0x8b5cf6));
    assert_eq!(sand.radius.large, 4.0);
    assert_eq!(sand.windows.app.corners.all(), [4.0; 4]);
    assert_eq!(
        catalog.entries().iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(),
        ["Light", "Dark", "Catppuccin Mocha", "Dracula", "Soft Sand"]
    );
}

#[test]
fn every_shipped_theme_is_opaque_without_blur_and_casts_the_same_soft_shadow() {
    let catalog = load(&[]);
    for entry in catalog.entries() {
        let theme = named(&catalog, &entry.id);
        for chrome in theme.windows.all() {
            assert_eq!(chrome.opacity, 1.0, "{}", entry.id);
            assert!(!chrome.blur, "{}", entry.id);
        }
        let strengths: Vec<f32> = theme.windows.app.shadow.iter().map(|layer| layer.color.a * layer.opacity).collect();
        assert_eq!(strengths.len(), 2, "{}", entry.id);
        assert!((strengths[0] - 0.0340).abs() < 1e-3, "{}: {strengths:?}", entry.id);
        assert!((strengths[1] - 0.0170).abs() < 1e-3, "{}: {strengths:?}", entry.id);
    }
}

#[test]
fn only_the_users_files_can_be_deleted() {
    let catalog = load(&[("mine", r#"{ "base": "dark" }"#)]);
    let deletable: Vec<&str> =
        catalog.entries().iter().filter(|entry| entry.deletable()).map(|entry| entry.id.as_str()).collect();
    assert_eq!(deletable, ["mine"]);
    assert!(catalog.reserved().iter().any(|id| id == "soft-sand"), "a new theme can't take a shipped name");
}

#[test]
fn a_file_named_like_a_shipped_theme_is_reported_and_the_shipped_one_stays() {
    let catalog = load(&[("dracula", r#"{ "base": "light" }"#)]);
    assert!(catalog.failures()[0].starts_with("dracula.json:"), "{:?}", catalog.failures());
    assert_eq!(named(&catalog, "dracula").launcher.background, color(0x282a36));
}

#[test]
fn broken_themes_are_reported_by_file_and_section() {
    let catalog = load(&[
        ("typo", r#"{ "base": "dark", "results": { "macth": "accent" } }"#),
        ("color", r#"{ "base": "dark", "palette": { "accent": "pink" } }"#),
        ("parent", r#"{ "base": "sepia" }"#),
        ("negative", r#"{ "base": "dark", "windows": { "app": { "corners": -4 } } }"#),
        ("dark", r#"{ "base": "light" }"#),
        ("partial", r##"{ "palette": { "text": "#fff" } }"##),
        ("glass", r##"{ "base": "dark", "results": { "selected": "#ffffff80" } }"##),
        ("haze", r#"{ "base": "dark", "windows": { "app": { "shadow": [{ "color": "shadow", "blur": -8 }] } } }"#),
    ]);
    let failures = catalog.failures();
    assert_eq!(failures.len(), 8, "{failures:?}");
    assert!(failures[0].starts_with("typo.json: results: unknown field `macth`"), "{}", failures[0]);
    assert!(failures[1].starts_with("color.json: palette.accent:"), "{}", failures[1]);
    assert!(failures[2].contains("unknown base `sepia`"), "{}", failures[2]);
    assert!(failures[3].contains("can't be negative"), "{}", failures[3]);
    assert!(failures[4].starts_with("dark.json:"), "{}", failures[4]);
    assert!(failures[5].starts_with("partial.json:"), "{}", failures[5]);
    assert!(failures[6].contains("must be opaque"), "{}", failures[6]);
    assert!(failures[7].contains("blur can't be negative"), "{}", failures[7]);

    assert!(catalog.entries().iter().all(|entry| !entry.deletable()), "none of the broken files loaded");
}

#[test]
fn a_missing_theme_falls_back_to_the_system_one() {
    let catalog = load(&[]);
    let none = ThemeEdits::default();
    let gone = catalog.resolve(&ThemeChoice::Named("deleted".to_owned()), Appearance::Dark, &none);
    assert_eq!(gone.launcher.background, color(0x000000));
    let system = catalog.resolve(&ThemeChoice::System, Appearance::Light, &none);
    assert_eq!(system.launcher.background, color(0xffffff));
}

#[test]
fn saved_theme_names_keep_their_meaning() {
    let old: ThemeChoice = serde_json::from_str(r#""dark""#).expect("json");
    assert_eq!(old, ThemeChoice::Named("dark".to_owned()));
    let system: ThemeChoice = serde_json::from_str(r#""system""#).expect("json");
    assert_eq!(system, ThemeChoice::System);
    assert_eq!(serde_json::to_string(&ThemeChoice::Named("dracula".to_owned())).expect("json"), r#""dracula""#);
}

#[test]
fn zoom_keeps_the_grid_on_whole_device_pixels() {
    for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
        assert_eq!(zoom_at(1.0, scale), 1.0);
        for zoom in [0.75, 1.1, 1.25, 1.5] {
            let step = GRID * zoom_at(zoom, scale) * scale;
            assert!((step - step.round()).abs() < 1e-4, "{zoom} at {scale}");
        }
    }
    assert_eq!(zoom_at(1.25, 1.0), 1.25);
    assert_eq!(zoom_at(1.25, 1.25), 1.2);
}

fn examples() -> Catalog {
    load(&[])
}

fn sheet(catalog: &Catalog, id: &str, edits: &ThemeEdits) -> Sheet {
    catalog.entries().iter().find(|entry| entry.id == id).expect("theme").sheet(edits)
}

fn edited(catalog: &Catalog, id: &str, edits: &ThemeEdits) -> Theme {
    catalog.resolve(&ThemeChoice::Named(id.to_owned()), Appearance::Light, edits)
}

#[test]
fn edits_change_only_their_own_theme() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    edits.set(&sheet(&catalog, "dark", &edits), &Slot::Palette("accent".into()), "#ff0080".into());
    edits.set(&sheet(&catalog, "dark", &edits), &Slot::Element("toggle", "on"), "accent".into());
    edits.set(&sheet(&catalog, "dark", &edits), &Slot::Radius("large"), number(10.0));
    let dark = edited(&catalog, "dark", &edits);
    assert_eq!(dark.results.matched, color(0xff0080), "elements on the role follow it");
    assert_eq!(dark.toggle.on, color(0xff0080));
    assert_eq!(dark.radius.large, 10.0);
    assert_eq!(dark.results.name, color(0xededed));
    assert_eq!(edited(&catalog, "light", &edits).results.matched, color(0x000000));
    assert_eq!(edited(&catalog, "dark", &ThemeEdits::default()).results.matched, color(0xffffff));
}

#[test]
fn an_edit_back_to_the_original_value_is_no_edit() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    let caret = Slot::Element("input", "caret");
    edits.set(&sheet(&catalog, "light", &edits), &caret, "muted".into());
    assert!(sheet(&catalog, "light", &edits).edited(&caret));
    edits.set(&sheet(&catalog, "light", &edits), &caret, "accent".into());
    assert_eq!(edits, ThemeEdits::default(), "nothing is kept for a theme without changes");

    let mocha = examples();
    let corners = serde_json::json!({ "top_left": 0, "top_right": 0, "bottom_right": 12, "bottom_left": 12 });
    edits.set(&sheet(&mocha, "catppuccin-mocha", &edits), &Slot::Corners(Pane::Dialog), corners);
    assert_eq!(edits, ThemeEdits::default(), "the same corners written another way");
    let layers = sheet(&mocha, "catppuccin-mocha", &edits).shadow(Pane::App);
    edits.set(&sheet(&mocha, "catppuccin-mocha", &edits), &Slot::Shadow(Pane::App), serde_json::json!(layers));
    assert_eq!(edits, ThemeEdits::default(), "the same shadow with every field spelled out");
}

#[test]
fn a_reset_brings_back_one_value_or_the_whole_theme() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    let opacity = Slot::Opacity(Pane::Launcher);
    let blur = Slot::Blur(Pane::Launcher);
    edits.set(&sheet(&catalog, "dark", &edits), &opacity, number(0.8));
    edits.set(&sheet(&catalog, "dark", &edits), &blur, true.into());
    edits.set(&sheet(&catalog, "light", &edits), &blur, true.into());
    let dark = sheet(&catalog, "dark", &edits);
    assert_eq!(dark.edits_under(&["windows"]), 2);
    assert_eq!(dark.number(&opacity), 0.8);
    edits.reset("dark", &opacity);
    let dark = sheet(&catalog, "dark", &edits);
    assert_eq!((dark.number(&opacity), dark.flag(&blur)), (1.0, true));
    edits.clear("dark");
    assert!(!sheet(&catalog, "dark", &edits).has_edits());
    assert!(sheet(&catalog, "light", &edits).flag(&blur), "other themes keep their changes");
}

#[test]
fn broken_edits_leave_the_theme_as_it_is() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    edits.set(&sheet(&catalog, "dark", &edits), &Slot::Element("results", "selected"), "#ffffff80".into());
    assert_eq!(edited(&catalog, "dark", &edits).results.selected, color(0x1c1c1c));
}

#[test]
fn a_saved_copy_keeps_its_roles_and_loads_as_it_looked() {
    let catalog = examples();
    let mut edits = ThemeEdits::default();
    edits.set(&sheet(&catalog, "dracula", &edits), &Slot::Palette("pink".into()), "#ff0000".into());
    edits.set(&sheet(&catalog, "dracula", &edits), &Slot::Corners(Pane::Launcher), number(12.0));
    let text = sheet(&catalog, "dracula", &edits).export("Мой Dracula");
    let copy = load(&[("copy", text.as_str())]);
    assert!(copy.failures().is_empty(), "{:?}", copy.failures());
    assert_eq!(copy.entries().last().map(|entry| entry.name.as_str()), Some("Мой Dracula"));
    assert_eq!(named(&copy, "copy"), edited(&catalog, "dracula", &edits));
    let source: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(source["base"], "dark");
    assert_eq!(source["palette"]["accent"], "pink", "roles stay references, not colors");
    assert!(text.trim_start().trim_start_matches('{').trim_start().starts_with(r#""name""#), "{text}");
    assert!(text.contains(r#""top_left": 12"#) || text.contains(r#""corners": 12"#), "{text}");
}

#[test]
fn a_palette_lists_only_the_standard_roles() {
    let dracula = sheet(&examples(), "dracula", &ThemeEdits::default());
    let names = dracula.palette();
    assert_eq!(names.iter().map(|name| name.as_ref()).collect::<Vec<_>>(), ROLES, "a theme's own color names stay out");
    assert_eq!(dracula.color(&Slot::Palette("text".into())), Some(color(0xf8f8f2)), "a role naming another color");
    assert_eq!(dracula.color(&Slot::Element("input", "caret")), Some(color(0xbd93f9)));
}

#[test]
fn the_editor_lists_every_element_of_the_design_system() {
    let base: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(include_str!("../../resources/themes/base.json")).expect("json");
    let groups: Vec<(&String, Vec<&str>)> = base
        .iter()
        .filter_map(|(key, value)| {
            let fields = value.as_object()?;
            fields
                .values()
                .all(serde_json::Value::is_string)
                .then(|| (key, fields.keys().map(String::as_str).collect()))
        })
        .collect();
    assert_eq!(groups.len(), GROUPS.len());
    for (key, mut fields) in groups {
        fields.sort_unstable();
        let group = GROUPS.iter().find(|group| group.key == key).unwrap_or_else(|| panic!("{key} is not editable"));
        let mut listed = group.fields.to_vec();
        listed.sort_unstable();
        assert_eq!(fields, listed, "{key}");
    }
    assert_eq!(base["radius"].as_object().map(serde_json::Map::len), Some(RADII.len()));
    let panes = Pane::ALL.map(Pane::key).to_vec();
    assert_eq!(base["windows"].as_object().map(|windows| windows.keys().map(String::as_str).collect()), Some(panes));
}

#[test]
fn hex_colors_round_trip() {
    assert_eq!(parse_hex("#ff0080"), Some(color(0xff0080)));
    assert_eq!(parse_hex("  abc "), Some(color(0xaabbcc)));
    assert_eq!(parse_hex("#ff008080"), None, "element colors are opaque");
    assert_eq!(parse_hex("+12345"), None);
    assert_eq!(parse_hex("#ggg"), None);
    assert_eq!(hex(color(0x1c1c1c)), "#1c1c1c");
    assert_eq!(parse_hex_alpha("#ff008080").map(hex_alpha).as_deref(), Some("#ff008080"));
    assert_eq!(parse_hex_alpha(" ff0080 "), Some(color(0xff0080)), "no alpha means opaque");
    assert_eq!(parse_hex_alpha("#ff0080gg"), None);
    assert_eq!(parse_hex_alpha("#ff00808"), None);
    assert_eq!(hex_alpha(color(0x000000).opacity(0.28)), "#00000047");
    assert_eq!(hex_alpha(color(0xffffff)), "#ffffff", "an opaque color keeps the short form");
    assert_eq!(number(0.28), serde_json::json!(0.28));
    assert_eq!(number(12.0), serde_json::json!(12));
}

#[test]
fn theme_names_become_safe_file_names() {
    assert_eq!(file_stem("Мой Dracula"), "Мой Dracula");
    assert_eq!(file_stem("  a/b:c?  "), "a-b-c-");
    assert_eq!(file_stem("..."), "theme");
    assert_eq!(file_stem("theme."), "theme");
    assert_eq!(file_stem("con"), "con-theme");
    assert_eq!(file_stem("COM1"), "COM1-theme");
    assert_eq!(file_stem("Console"), "Console");
}

#[test]
fn one_accent_recolors_every_accent_element_and_resets_together() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    edits.apply(&sheet(&catalog, "light", &edits), &accent("#3b82f6"));
    let light = edited(&catalog, "light", &edits);
    for shown in [light.results.matched, light.input.caret, light.toggle.on, light.slider.fill, light.dropdown.check] {
        assert_eq!(shown, color(0x3b82f6));
    }
    assert_eq!(light.results.name, color(0x171717), "plain text keeps its color");
    let shown = sheet(&catalog, "light", &edits);
    assert!(shown.wears(&accent("#3B82F6")) && !shown.wears(&accent("#ef4444")));
    assert_eq!(shown.original_color(&Slot::Palette("accent".into())), Some(color(0x000000)));
    edits.reset_all("light", &accent_slots());
    assert_eq!(edits, ThemeEdits::default());
}

#[test]
fn a_rounding_level_reaches_controls_and_only_the_outer_window_corners() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    edits.apply(&sheet(&catalog, "dark", &edits), &Roundness::Round.look());
    let dark = edited(&catalog, "dark", &edits);
    assert_eq!((dark.radius.large, dark.radius.small), (10.0, 6.0));
    assert_eq!(dark.windows.launcher.corners.all(), [12.0; 4]);
    assert_eq!(dark.windows.dialog.corners.all(), [0.0, 0.0, 12.0, 12.0], "the bar hangs under the dialog");
    assert_eq!(dark.windows.explorer.corners.all(), [12.0, 0.0, 0.0, 0.0], "the bar sits in the window corner");
    let shown = sheet(&catalog, "dark", &edits);
    assert!(shown.wears(&Roundness::Round.look()) && !shown.wears(&Roundness::Medium.look()));
    assert!(!sheet(&catalog, "dark", &ThemeEdits::default()).wears(&Roundness::Square.look()));
}

#[test]
fn transparency_and_blur_switch_separately_on_every_window() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    edits.apply(&sheet(&catalog, "dark", &edits), &Clarity::Medium.look());
    let windows = edited(&catalog, "dark", &edits).windows;
    assert!(windows.all().iter().all(|chrome| chrome.opacity == 0.84 && !chrome.blur));
    assert!(!blur_on(&sheet(&catalog, "dark", &edits)));
    edits.apply(&sheet(&catalog, "dark", &edits), &blur(true));
    let windows = edited(&catalog, "dark", &edits).windows;
    assert!(windows.all().iter().all(|chrome| chrome.opacity == 0.84 && chrome.blur));
    edits.apply(&sheet(&catalog, "dark", &edits), &blur(false));
    edits.apply(&sheet(&catalog, "dark", &edits), &Clarity::Solid.look());
    assert_eq!(edits, ThemeEdits::default(), "solid without blur is how the built-in themes come");
}

#[test]
fn a_translucent_shadow_role_thins_every_shadow_layer() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    edits.set(&sheet(&catalog, "light", &edits), &Slot::Palette("shadow".into()), "#00000080".into());
    let layers = edited(&catalog, "light", &edits).windows.app.shadow;
    assert!(!layers.is_empty());
    assert!(layers.iter().all(|layer| (layer.color.a - 128.0 / 255.0).abs() < 1e-3), "{layers:?}");
}

#[test]
fn shadows_switch_off_and_come_back_as_the_theme_had_them() {
    let catalog = load(&[("flat", r#"{ "base": "dark", "windows": { "launcher": { "shadow": [] } } }"#)]);
    let mut edits = ThemeEdits::default();
    edits.apply(&sheet(&catalog, "dark", &edits), &no_shadows());
    assert!(!shadows_on(&sheet(&catalog, "dark", &edits)));
    let back = shadows(&sheet(&catalog, "dark", &edits));
    edits.apply(&sheet(&catalog, "dark", &edits), &back);
    assert_eq!(edits, ThemeEdits::default());
    let flat = sheet(&catalog, "flat", &edits);
    let lit = shadows(&flat);
    edits.apply(&flat, &lit);
    let windows = edited(&catalog, "flat", &edits).windows;
    assert_eq!(windows.launcher.shadow.len(), 2, "a window without a shadow gets the default one");
    assert!(windows.dialog.shadow.is_empty(), "the docked bar stays flat");
}

fn near(left: Hsla, right: Hsla) -> bool {
    let (left, right) = (left.to_rgb(), right.to_rgb());
    [(left.r, right.r), (left.g, right.g), (left.b, right.b)].iter().all(|(a, b)| (a - b).abs() <= 6.0 / 255.0)
}

#[test]
fn a_background_alone_brings_a_whole_matching_palette() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    let dark = sheet(&catalog, "dark", &edits);
    let white = color(0xffffff);
    edits.apply(&dark, &colors(&dark, white, text_for(white)));
    let made = edited(&catalog, "dark", &edits);
    let light = named(&catalog, "light");
    for (got, want, role) in [
        (made.results.name, light.results.name, "text"),
        (made.results.path, light.results.path, "muted"),
        (made.results.selected, light.results.selected, "selected"),
        (made.button.frame, light.button.frame, "border"),
        (made.settings.divider, light.settings.divider, "divider"),
    ] {
        assert!(near(got, want), "{role}: {got:?} vs {want:?}");
    }
    assert_eq!(made.results.matched, made.results.name, "the white accent of the dark theme can't be read on white");
}

#[test]
fn text_stays_readable_on_any_background() {
    for background in [0xffffff, 0xf6f3ec, 0x1f1f23, 0x000000, 0x0f172a, 0x808080] {
        let background = color(background);
        assert!(contrast(text_for(background), background) >= 4.5, "{background:?}");
    }
}

#[test]
fn a_readable_accent_survives_a_new_background() {
    let catalog = examples();
    let mut edits = ThemeEdits::default();
    let mocha = sheet(&catalog, "catppuccin-mocha", &edits);
    let navy = color(0x0f172a);
    edits.apply(&mocha, &colors(&mocha, navy, text_for(navy)));
    let made = edited(&catalog, "catppuccin-mocha", &edits);
    assert_eq!(made.results.matched, color(0xcba6f7));
    assert_eq!(made.launcher.background, navy);
    assert_eq!(made.nav.current_text, made.results.name, "the lavender of the current item gives way to the text");
    edits.reset_all("catppuccin-mocha", &color_slots(&mocha));
    assert_eq!(edits, ThemeEdits::default());
}

#[test]
fn an_accent_picked_by_hand_is_kept_and_a_theme_accent_follows_the_text() {
    let catalog = load(&[]);
    let mut edits = ThemeEdits::default();
    let navy = color(0x0f172a);
    edits.apply(&sheet(&catalog, "light", &edits), &colors(&sheet(&catalog, "light", &edits), navy, text_for(navy)));
    let shown = sheet(&catalog, "light", &edits);
    assert!(!accent_chosen(&shown));
    assert_eq!(theme_accent(&shown), Some(text_for(navy)).map(|text| parse_hex(&hex(text)).expect("hex")));
    assert_eq!(edited(&catalog, "light", &edits).input.caret, edited(&catalog, "light", &edits).results.name);
    edits.apply(&shown, &accent("#ef4444"));
    let paper = color(0xf6f3ec);
    let picked = sheet(&catalog, "light", &edits);
    edits.apply(&picked, &colors(&picked, paper, text_for(paper)));
    assert_eq!(edited(&catalog, "light", &edits).input.caret, color(0xef4444), "a chosen accent stays");
}

#[test]
fn offered_presets_skip_the_theme_color_and_keep_their_count() {
    let presets = offered(&ACCENTS, Some(color(0x8b5cf6)));
    let colors: Vec<&str> = presets.iter().map(|(_, color)| *color).collect();
    assert_eq!(colors, ["#3b82f6", "#ec4899", "#ef4444", "#f97316", "#22c55e", "#14b8a6", "#f59e0b"]);
    assert_eq!(presets.get(1).map(|(index, _)| *index), Some(2), "a preset keeps its own index");
    let unrelated = offered(&BACKGROUNDS, Some(color(0x282a36)));
    assert_eq!(unrelated.len(), 7);
    assert!(unrelated.iter().all(|(_, color)| *color != "#002b36"), "the spare waits while nothing repeats");
}

#[test]
fn a_new_background_keeps_the_theme_accent_and_a_preset_accent_can_still_be_picked() {
    let catalog = examples();
    let mut edits = ThemeEdits::default();
    let sand = sheet(&catalog, "soft-sand", &edits);
    let white = color(0xffffff);
    edits.apply(&sand, &colors(&sand, white, text_for(white)));
    let shown = sheet(&catalog, "soft-sand", &edits);
    assert!(!accent_chosen(&shown), "a background is not an accent");
    assert_eq!(edited(&catalog, "soft-sand", &edits).toggle.on, color(0x8b5cf6));
    edits.apply(&shown, &accent("#3b82f6"));
    let picked = sheet(&catalog, "soft-sand", &edits);
    assert!(accent_chosen(&picked) && picked.wears(&accent("#3b82f6")));
    assert_eq!(edited(&catalog, "soft-sand", &edits).toggle.on, color(0x3b82f6));
}
