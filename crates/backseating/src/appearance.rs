//! Light or dark, in which colours and fonts: the engine Aiddiction and
//! matterfast use, `gpui-adaptive-colors` and `gpui-zed-themes`. This module
//! only remembers the choices in the settings and hands them to it.
//!
//! - The colours are a Material You scheme made from the system's colour or
//!   one picked here (`set_color`).
//! - Instead of the scheme a theme for the Zed editor can be worn, one for
//!   light and one for dark, installed from Zed's registry with the library's
//!   `Browser` into the themes directory.
//! - Light or dark is the toolkit's own `Theme::change`, following the
//!   desktop unless one is chosen.
//! - The interface and code fonts go on top of whatever theme is shown, and
//!   again whenever one is shown, since a theme comes with fonts of its own.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

use gpui_adaptive_colors::{AdaptiveColors, Color, Options, Seed, Source};
use gpui_kit::component::{Theme, ThemeConfig, ThemeMode};
use gpui_kit::{App, Global, Pixels, SharedString, Window, WindowAppearance, px};
use gpui_zed_themes::{Registry, Store};

use crate::settings::Settings;

/// The colours offered beside the system's.
pub const SEEDS: [(&str, &str); 6] = [
    ("#0b57d0", "Blue"),
    ("#00838f", "Teal"),
    ("#2fb380", "Green"),
    ("#6750a4", "Violet"),
    ("#e91e63", "Pink"),
    ("#c88800", "Amber"),
];

/// What the scheme is called in a list of themes.
pub const SCHEME: &str = "Color scheme";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    System,
    Light,
    Dark,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::System, Mode::Light, Mode::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Mode::System => "System",
            Mode::Light => "Light",
            Mode::Dark => "Dark",
        }
    }
}

/// Which of the two fonts a choice is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontRole {
    /// Everything that is not code.
    Interface,
    /// The diff.
    Code,
}

/// The installed Zed themes, read at startup and after an install.
struct Installed(Vec<Rc<ThemeConfig>>);

impl Global for Installed {}

fn themes_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("backseating")
        .join("themes")
}

pub fn store() -> Store {
    Store::new(themes_dir())
}

/// Zed's registry, or `BACKSEATING_THEMES_API` in its place for tests.
pub fn registry() -> Registry {
    let registry = Registry::new(concat!("backseating/", env!("CARGO_PKG_VERSION")));
    match std::env::var("BACKSEATING_THEMES_API") {
        Ok(api) if !api.is_empty() => registry.at(api),
        _ => registry,
    }
}

/// Starts the engine with the stored choices. Once, before the first window.
pub fn init(cx: &mut App) {
    // Before any theme is shown: the fonts in force now are the built-in ones.
    built_in_fonts(cx);
    let remembered = Settings::get(cx)
        .appearance
        .system_color
        .as_deref()
        .and_then(|seed| seed.parse::<Seed>().ok());
    gpui_adaptive_colors::init(
        Options::new(Color::from_u32(0x0b57d0))
            .source(source(cx))
            .remembered(remembered),
        cx,
    );
    // The system's colour arrives after the first frame; kept, it is what the
    // next launch starts with.
    cx.observe_global::<AdaptiveColors>(|cx| {
        let said = AdaptiveColors::global(cx)
            .system_seed()
            .map(|seed| seed.to_string());
        if Settings::get(cx).appearance.system_color != said {
            Settings::update(cx, |settings| settings.appearance.system_color = said);
        }
    })
    .detach();
    // A theme shown again — a change of mode, of colour — is shown in its
    // own fonts; the chosen ones go back on.
    cx.observe_global::<Theme>(apply_fonts).detach();
    reload_themes(cx);
    apply_mode(None, cx);
}

/// `BACKSEATING_COLOR=#rrggbb` pins the colour, so a screenshot does not
/// depend on the desktop's.
fn source(cx: &App) -> Source {
    std::env::var("BACKSEATING_COLOR")
        .ok()
        .or_else(|| Settings::get(cx).appearance.color.clone())
        .and_then(|stored| Color::parse_hex(&stored))
        .map_or(Source::System, Source::from)
}

/// The colour chosen, or `None` for the system's.
pub fn color(cx: &App) -> Option<Color> {
    Settings::get(cx)
        .appearance
        .color
        .as_deref()
        .and_then(Color::parse_hex)
}

pub fn set_color(color: Option<Color>, cx: &mut App) {
    Settings::update(cx, |settings| {
        settings.appearance.color = color.map(|color| color.to_hex())
    });
    gpui_adaptive_colors::set_source(source(cx), cx);
}

pub fn mode(cx: &App) -> Mode {
    match Settings::get(cx).appearance.mode.as_deref() {
        Some("light") => Mode::Light,
        Some("dark") => Mode::Dark,
        _ => Mode::System,
    }
}

pub fn set_mode(mode: Mode, window: &mut Window, cx: &mut App) {
    let stored = match mode {
        Mode::System => None,
        Mode::Light => Some("light".to_owned()),
        Mode::Dark => Some("dark".to_owned()),
    };
    Settings::update(cx, |settings| settings.appearance.mode = stored);
    apply_mode(Some(window), cx);
}

/// Light or dark as chosen, or as the desktop is. Again whenever the
/// window's appearance changes; on Linux only a window knows what it is.
pub fn apply_mode(window: Option<&mut Window>, cx: &mut App) {
    let mode = match mode(cx) {
        Mode::Light => ThemeMode::Light,
        Mode::Dark => ThemeMode::Dark,
        Mode::System => {
            let appearance =
                window.map_or_else(|| cx.window_appearance(), |window| window.appearance());
            match appearance {
                WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
                WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
            }
        }
    };
    if Theme::global(cx).mode != mode {
        Theme::change(mode, None, cx);
    }
}

/// Reads the installed themes again and wears what the settings name.
pub fn reload_themes(cx: &mut App) {
    let themes = store().themes().into_iter().map(Rc::new).collect();
    cx.set_global(Installed(themes));
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        match chosen(mode, cx) {
            Some(theme) => gpui_adaptive_colors::wear(theme, cx),
            None => gpui_adaptive_colors::wear_scheme(mode, cx),
        }
    }
}

fn chosen(mode: ThemeMode, cx: &App) -> Option<Rc<ThemeConfig>> {
    let appearance = &Settings::get(cx).appearance;
    let wanted = if mode.is_dark() {
        appearance.dark_theme.as_deref()
    } else {
        appearance.light_theme.as_deref()
    }?;
    cx.try_global::<Installed>()?
        .0
        .iter()
        .find(|theme| theme.mode == mode && theme.name.as_ref() == wanted)
        .cloned()
}

/// The scheme first, then the installed themes of that mode by name.
pub fn theme_names(mode: ThemeMode, cx: &App) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = cx
        .try_global::<Installed>()
        .map(|installed| {
            installed
                .0
                .iter()
                .filter(|theme| theme.mode == mode)
                .map(|theme| theme.name.clone())
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names.insert(0, SCHEME.into());
    names
}

/// What is worn in `mode`, as [`theme_names`] lists it.
pub fn worn(mode: ThemeMode, cx: &App) -> SharedString {
    chosen(mode, cx).map_or(SCHEME.into(), |theme| theme.name.clone())
}

pub fn choose_theme(mode: ThemeMode, name: &str, cx: &mut App) {
    let name = (name != SCHEME).then(|| name.to_owned());
    Settings::update(cx, |settings| {
        if mode.is_dark() {
            settings.appearance.dark_theme = name;
        } else {
            settings.appearance.light_theme = name;
        }
    });
    reload_themes(cx);
}

/// The fonts in force before anything was chosen: (interface, code), and
/// their sizes.
static BUILT_IN: OnceLock<BuiltIn> = OnceLock::new();

struct BuiltIn {
    families: (SharedString, SharedString),
    sizes: (Pixels, Pixels),
}

fn built_in(cx: &App) -> &'static BuiltIn {
    BUILT_IN.get_or_init(|| {
        let theme = Theme::global(cx);
        BuiltIn {
            families: (theme.font_family.clone(), theme.mono_font_family.clone()),
            sizes: (theme.font_size, theme.mono_font_size),
        }
    })
}

pub fn built_in_fonts(cx: &App) -> (SharedString, SharedString) {
    built_in(cx).families.clone()
}

/// The sizes a font size may be set to, in pixels, by role.
pub fn font_sizes(role: FontRole) -> std::ops::RangeInclusive<u16> {
    match role {
        FontRole::Interface => 11..=24,
        FontRole::Code => 9..=32,
    }
}

/// The size a role's text is drawn at, in whole pixels.
pub fn font_size(role: FontRole, cx: &App) -> u16 {
    let appearance = &Settings::get(cx).appearance;
    let (chosen, built_in) = match role {
        FontRole::Interface => (appearance.ui_font_size, built_in(cx).sizes.0),
        FontRole::Code => (appearance.code_font_size, built_in(cx).sizes.1),
    };
    chosen
        .unwrap_or(f32::from(built_in).round() as u16)
        .clamp(*font_sizes(role).start(), *font_sizes(role).end())
}

/// Whether a role's size is the built-in one.
pub fn font_size_is_default(role: FontRole, cx: &App) -> bool {
    let appearance = &Settings::get(cx).appearance;
    match role {
        FontRole::Interface => appearance.ui_font_size.is_none(),
        FontRole::Code => appearance.code_font_size.is_none(),
    }
}

/// `None` goes back to the built-in size.
pub fn set_font_size(role: FontRole, size: Option<u16>, cx: &mut App) {
    let size = size.map(|size| size.clamp(*font_sizes(role).start(), *font_sizes(role).end()));
    Settings::update(cx, |settings| match role {
        FontRole::Interface => settings.appearance.ui_font_size = size,
        FontRole::Code => settings.appearance.code_font_size = size,
    });
    apply_fonts(cx);
}

/// The family chosen for a role, or nothing for the built-in one.
pub fn font_choice(role: FontRole, cx: &App) -> Option<String> {
    let appearance = &Settings::get(cx).appearance;
    match role {
        FontRole::Interface => appearance.ui_font.clone(),
        FontRole::Code => appearance.code_font.clone(),
    }
    .filter(|family| !family.is_empty())
}

/// `None` goes back to the built-in font.
pub fn set_font_choice(role: FontRole, family: Option<String>, cx: &mut App) {
    Settings::update(cx, |settings| match role {
        FontRole::Interface => settings.appearance.ui_font = family,
        FontRole::Code => settings.appearance.code_font = family,
    });
    apply_fonts(cx);
}

/// Makes the theme's fonts the chosen ones, at the chosen sizes. A font
/// uninstalled since it was chosen is not used; the setting stays, in case it
/// comes back.
pub fn apply_fonts(cx: &mut App) {
    let built_in = built_in_fonts(cx);
    let sizes = (
        px(f32::from(font_size(FontRole::Interface, cx))),
        px(f32::from(font_size(FontRole::Code, cx))),
    );
    let known = cx.text_system().all_font_names();
    let pick = |role: FontRole, fallback: &SharedString| -> SharedString {
        font_choice(role, cx)
            .filter(|family| known.iter().any(|name| name == family))
            .map(SharedString::from)
            .unwrap_or_else(|| fallback.clone())
    };
    let interface = pick(FontRole::Interface, &built_in.0);
    let code = pick(FontRole::Code, &built_in.1);
    let theme = Theme::global(cx);
    if theme.font_family != interface
        || theme.mono_font_family != code
        || (theme.font_size, theme.mono_font_size) != sizes
    {
        Theme::update(cx, |theme| {
            theme.font_family = interface;
            theme.mono_font_family = code;
            // The interface's size is the window's rem: everything sized
            // in rems, which is nearly everything, follows it.
            theme.font_size = sizes.0;
            theme.mono_font_size = sizes.1;
        });
    }
}
