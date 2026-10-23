//! Look-and-feel preferences: palette, pane style, accent, text size and the
//! side panel, saved next to the other preferences and applied to the theme.

use std::path::PathBuf;

use std::rc::Rc;

use serde::{Deserialize, Serialize};
use slint::{Color, ComponentHandle, ModelRc, VecModel};

use crate::core::core;
use crate::ui::{AppWindow, PaletteChoice, PaneStyle, SideTab, Theme};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Panes {
    #[default]
    Minimal,
    Boxed,
    Glass,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum FontSize {
    Small,
    #[default]
    Normal,
    Large,
}

impl FontSize {
    pub const ALL: [FontSize; 3] = [FontSize::Small, FontSize::Normal, FontSize::Large];

    fn px(self) -> f32 {
        match self {
            FontSize::Small => 12.0,
            FontSize::Normal => 13.0,
            FontSize::Large => 15.0,
        }
    }
}

/// What the right-hand panel shows, if anything.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SidePanel {
    #[default]
    Lyrics,
    Queue,
    Hidden,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub palette: String,
    pub pane_style: Panes,
    /// Overrides the palette's accent when set ("#rrggbb").
    pub accent: Option<String>,
    /// Take the accent and background glow from the playing track's cover.
    pub cover_colors: bool,
    pub font_size: FontSize,
    pub side_panel: SidePanel,
    /// The colours of the "custom" palette.
    pub custom_palette: CustomPalette,
}

/// The user's own palette: three colours ("#rrggbb"); the rest are mixed from them.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct CustomPalette {
    pub bg: String,
    pub fg: String,
    pub accent: String,
}

impl Default for CustomPalette {
    fn default() -> Self {
        Self {
            bg: "#101418".into(),
            fg: "#e6e6e6".into(),
            accent: "#4fd1c5".into(),
        }
    }
}

pub const CUSTOM: &str = "custom";

impl CustomPalette {
    fn palette(&self) -> Palette {
        let defaults = CustomPalette::default();
        let color = |text: &str, default: &str| parse_hex(text).or_else(|| parse_hex(default)).unwrap_or(0);
        let bg = color(&self.bg, &defaults.bg);
        let fg = color(&self.fg, &defaults.fg);
        let accent = color(&self.accent, &defaults.accent);
        Palette {
            id: CUSTOM,
            name: "Custom",
            bg,
            fg,
            muted: mix(bg, fg, 0.68),
            dim: mix(bg, fg, 0.38),
            border: mix(bg, fg, 0.14),
            hover: mix(bg, fg, 0.05),
            select: mix(bg, fg, 0.09),
            accent,
            glow: (accent, 0.25),
            glow2: (accent, 0.15),
            error: 0xff6b6b,
        }
    }
}

/// `a` moved `t` of the way to `b`, per channel.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            palette: "amber".into(),
            pane_style: Panes::Minimal,
            accent: None,
            cover_colors: false,
            font_size: FontSize::Normal,
            side_panel: SidePanel::Lyrics,
            custom_palette: CustomPalette::default(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub id: &'static str,
    pub name: &'static str,
    bg: u32,
    fg: u32,
    muted: u32,
    dim: u32,
    border: u32,
    hover: u32,
    select: u32,
    accent: u32,
    /// The glass style's two background glows, as 0xRRGGBB and opacity.
    glow: (u32, f32),
    glow2: (u32, f32),
    error: u32,
}

pub const PALETTES: [Palette; 11] = [
    Palette {
        id: "amber",
        name: "Amber",
        bg: 0x161616,
        fg: 0xe8e2d6,
        muted: 0xa09a8f,
        dim: 0x5f5b54,
        border: 0x2c2a27,
        hover: 0x1f1e1c,
        select: 0x262420,
        accent: 0xf5a524,
        glow: (0xf5a524, 0.22),
        glow2: (0xb44628, 0.25),
        error: 0xff6b6b,
    },
    Palette {
        id: "catppuccin",
        name: "Catppuccin",
        bg: 0x1e1e2e,
        fg: 0xcdd6f4,
        muted: 0xa6adc8,
        dim: 0x6c7086,
        border: 0x45475a,
        hover: 0x28283b,
        select: 0x313244,
        accent: 0xcba6f7,
        glow: (0xcba6f7, 0.3),
        glow2: (0xfab387, 0.22),
        error: 0xf38ba8,
    },
    Palette {
        id: "tokyo",
        name: "Tokyo Night",
        bg: 0x0d1020,
        fg: 0xe4e8ff,
        muted: 0xa3acd6,
        dim: 0x6871a3,
        border: 0x283057,
        hover: 0x161b33,
        select: 0x1c2444,
        accent: 0x7aa2f7,
        glow: (0x8b5cf6, 0.4),
        glow2: (0xf28d6b, 0.3),
        error: 0xf7768e,
    },
    Palette {
        id: "nord",
        name: "Nord",
        bg: 0x2e3440,
        fg: 0xeceff4,
        muted: 0xc0c8d6,
        dim: 0x7b879c,
        border: 0x434c5e,
        hover: 0x353c4a,
        select: 0x3b4252,
        accent: 0x88c0d0,
        glow: (0x88c0d0, 0.25),
        glow2: (0xb48ead, 0.25),
        error: 0xbf616a,
    },
    Palette {
        id: "gruvbox",
        name: "Gruvbox",
        bg: 0x1d2021,
        fg: 0xebdbb2,
        muted: 0xbdae93,
        dim: 0x7c6f64,
        border: 0x3c3836,
        hover: 0x282828,
        select: 0x32302f,
        accent: 0xb8bb26,
        glow: (0xb8bb26, 0.2),
        glow2: (0xd65d0e, 0.25),
        error: 0xfb4934,
    },
    Palette {
        id: "dracula",
        name: "Dracula",
        bg: 0x1e1f29,
        fg: 0xf8f8f2,
        muted: 0xc0c3d6,
        dim: 0x6272a4,
        border: 0x373a4d,
        hover: 0x262836,
        select: 0x2e3040,
        accent: 0xbd93f9,
        glow: (0xbd93f9, 0.3),
        glow2: (0xff79c6, 0.22),
        error: 0xff5555,
    },
    Palette {
        id: "rosepine",
        name: "Rosé Pine",
        bg: 0x191724,
        fg: 0xe0def4,
        muted: 0xb3afd0,
        dim: 0x6e6a86,
        border: 0x2f2b44,
        hover: 0x1f1d2e,
        select: 0x26233a,
        accent: 0xebbcba,
        glow: (0xc4a7e7, 0.3),
        glow2: (0xebbcba, 0.22),
        error: 0xeb6f92,
    },
    Palette {
        id: "solarized",
        name: "Solarized",
        bg: 0x002b36,
        fg: 0xeee8d5,
        muted: 0x93a1a1,
        dim: 0x657b83,
        border: 0x0f4250,
        hover: 0x053542,
        select: 0x073642,
        accent: 0x2aa198,
        glow: (0x268bd2, 0.3),
        glow2: (0xb58900, 0.22),
        error: 0xdc322f,
    },
    Palette {
        id: "onedark",
        name: "One Dark",
        bg: 0x21252b,
        fg: 0xdcdfe4,
        muted: 0xabb2bf,
        dim: 0x5c6370,
        border: 0x363b45,
        hover: 0x282c34,
        select: 0x2c313a,
        accent: 0x61afef,
        glow: (0x61afef, 0.25),
        glow2: (0xc678dd, 0.22),
        error: 0xe06c75,
    },
    Palette {
        id: "everforest",
        name: "Everforest",
        bg: 0x272e33,
        fg: 0xd3c6aa,
        muted: 0xa9b09a,
        dim: 0x7a8478,
        border: 0x414b50,
        hover: 0x2e383c,
        select: 0x374145,
        accent: 0xa7c080,
        glow: (0xa7c080, 0.22),
        glow2: (0xe69875, 0.22),
        error: 0xe67e80,
    },
    Palette {
        id: "kanagawa",
        name: "Kanagawa",
        bg: 0x1a1a22,
        fg: 0xdcd7ba,
        muted: 0xb0ac93,
        dim: 0x727169,
        border: 0x363646,
        hover: 0x21212b,
        select: 0x2a2a37,
        accent: 0x7e9cd8,
        glow: (0x957fb8, 0.3),
        glow2: (0xffa066, 0.22),
        error: 0xe82424,
    },
];

pub const ACCENTS: [&str; 7] = ["#f5a524", "#ff6b6b", "#f472b6", "#cba6f7", "#7aa2f7", "#5eead4", "#a3e635"];

impl Palette {
    pub fn swatch(&self) -> [Color; 3] {
        [rgb(self.bg), rgb(self.fg), rgb(self.accent)]
    }
}

fn rgb(hex: u32) -> Color {
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn rgba((hex, alpha): (u32, f32)) -> Color {
    rgb(hex).with_alpha(alpha)
}

/// Parses "#rrggbb".
pub fn parse_color(text: &str) -> Option<Color> {
    parse_hex(text).map(rgb)
}

fn parse_hex(text: &str) -> Option<u32> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// "#rrggbb" for a colour typed in any case, or None if it isn't one.
fn normalize_hex(text: &str) -> Option<String> {
    parse_hex(text).map(|hex| format!("#{hex:06x}"))
}

fn path() -> PathBuf {
    core().config_dir.join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read(path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let _ = std::fs::create_dir_all(&core().config_dir);
    if let Ok(json) = serde_json::to_vec_pretty(settings) {
        // Settings just won't be remembered if this fails.
        let _ = std::fs::write(path(), json);
    }
}

pub fn palette(settings: &Settings) -> Palette {
    if settings.palette == CUSTOM {
        return settings.custom_palette.palette();
    }
    PALETTES.iter().find(|p| p.id == settings.palette).copied().unwrap_or(PALETTES[0])
}

/// Colours taken from the playing cover: an accent and a glow.
#[derive(Clone, Copy)]
pub struct CoverColors {
    pub accent: Color,
    pub glow: Color,
}

/// Picks a vivid colour from RGBA pixels: averages them, weighting saturated
/// and mid-bright ones more, so a mostly black cover still yields its accent.
pub fn cover_colors(rgba_pixels: &[u8]) -> Option<CoverColors> {
    let (mut r, mut g, mut b, mut total) = (0.0, 0.0, 0.0, 0.0);
    for pixel in rgba_pixels.as_chunks::<4>().0 {
        let (pr, pg, pb) = (pixel[0] as f32, pixel[1] as f32, pixel[2] as f32);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        let saturation = if max == 0.0 { 0.0 } else { (max - min) / max };
        let brightness = max / 255.0;
        let weight = 0.05 + saturation * saturation * (1.0 - (brightness - 0.65).abs());
        r += pr * weight;
        g += pg * weight;
        b += pb * weight;
        total += weight;
    }
    if total == 0.0 {
        return None;
    }
    let (r, g, b) = (r / total, g / total, b / total);
    // Lift dark colours so the accent stays readable on a dark background.
    let max = r.max(g).max(b).max(1.0);
    let lift = if max < 200.0 { 200.0 / max } else { 1.0 };
    let channel = |v: f32| (v * lift).min(255.0).round() as u8;
    Some(CoverColors {
        accent: Color::from_rgb_u8(channel(r), channel(g), channel(b)),
        glow: Color::from_argb_u8((0.45 * 255.0) as u8, r as u8, g as u8, b as u8),
    })
}

/// Sets the theme from the settings, with the cover's colours when they're on.
pub fn apply(window: &AppWindow, settings: &Settings, cover: Option<CoverColors>) {
    let palette = palette(settings);
    let theme = window.global::<Theme>();
    let cover = cover.filter(|_| settings.cover_colors);
    let accent = cover
        .map(|c| c.accent)
        .or_else(|| settings.accent.as_deref().and_then(parse_color))
        .unwrap_or(rgb(palette.accent));
    theme.set_bg(rgb(palette.bg));
    theme.set_fg(rgb(palette.fg));
    theme.set_muted(rgb(palette.muted));
    theme.set_dim(rgb(palette.dim));
    theme.set_border(rgb(palette.border));
    theme.set_hover(rgb(palette.hover));
    theme.set_select(rgb(palette.select));
    theme.set_error(rgb(palette.error));
    theme.set_accent(accent);
    theme.set_glow(cover.map(|c| c.glow).unwrap_or(rgba(palette.glow)));
    theme.set_glow2(rgba(palette.glow2));
    theme.set_font_size(settings.font_size.px());
    let (style, radius) = match settings.pane_style {
        Panes::Minimal => (PaneStyle::Minimal, 0.0),
        Panes::Boxed => (PaneStyle::Boxed, 10.0),
        Panes::Glass => (PaneStyle::Glass, 16.0),
    };
    theme.set_pane_style(style);
    theme.set_radius(radius);
}

struct State {
    settings: Settings,
    cover: Option<CoverColors>,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

/// The current settings.
pub fn get() -> Settings {
    STATE.with_borrow(|s| s.as_ref().map(|s| s.settings.clone()).unwrap_or_default())
}

/// Changes the settings, saves them and redraws with them.
pub fn update(window: &AppWindow, f: impl FnOnce(&mut Settings)) {
    let changed = STATE.with_borrow_mut(|s| {
        let state = s.as_mut()?;
        let before = state.settings.clone();
        f(&mut state.settings);
        (state.settings != before).then(|| (state.settings.clone(), state.cover))
    });
    if let Some((settings, cover)) = changed {
        save(&settings);
        show(window, &settings, cover);
    }
}

/// The playing cover's colours, for the "take colours from the cover" setting.
pub fn set_cover(window: &AppWindow, cover: Option<CoverColors>) {
    let settings = STATE.with_borrow_mut(|s| {
        let state = s.as_mut()?;
        state.cover = cover;
        Some(state.settings.clone())
    });
    if let Some(settings) = settings {
        show(window, &settings, cover);
    }
}

/// Loads the settings and wires the settings panel; once, before the window shows.
pub fn init(window: &AppWindow) {
    let settings = load();
    STATE.with_borrow_mut(|s| {
        *s = Some(State {
            settings: settings.clone(),
            cover: None,
        })
    });
    let accents: Vec<Color> = ACCENTS.iter().filter_map(|a| parse_color(a)).collect();
    window.set_accents(ModelRc::from(Rc::new(VecModel::from(accents))));
    show(window, &settings, None);

    window.on_choose_palette(|index| {
        let id = PALETTES.get(index as usize).map_or(CUSTOM, |p| p.id);
        with_window(move |w| update(w, |s| s.palette = id.into()))
    });
    window.on_choose_custom_color(|which, text| {
        let Some(hex) = normalize_hex(&text) else { return };
        with_window(move |w| {
            update(w, |s| {
                let custom = &mut s.custom_palette;
                match which {
                    0 => custom.bg = hex,
                    1 => custom.fg = hex,
                    _ => custom.accent = hex,
                }
                s.palette = CUSTOM.into();
            })
        })
    });
    window.on_choose_panes(|style| {
        let panes = match style {
            PaneStyle::Minimal => Panes::Minimal,
            PaneStyle::Boxed => Panes::Boxed,
            PaneStyle::Glass => Panes::Glass,
        };
        with_window(move |w| update(w, |s| s.pane_style = panes))
    });
    window.on_choose_accent(|index| {
        let accent = usize::try_from(index).ok().and_then(|i| ACCENTS.get(i)).map(|a| a.to_string());
        with_window(move |w| update(w, |s| s.accent = accent))
    });
    window.on_choose_custom_accent(|text| {
        if let Some(hex) = normalize_hex(&text) {
            with_window(move |w| update(w, |s| s.accent = Some(hex)));
        }
    });
    window.on_toggle_cover_colors(|| with_window(|w| update(w, |s| s.cover_colors = !s.cover_colors)));
    window.on_choose_font_size(|index| {
        let size = FontSize::ALL[index as usize % FontSize::ALL.len()];
        with_window(move |w| update(w, |s| s.font_size = size))
    });
}

fn with_window(f: impl FnOnce(&AppWindow)) {
    if let Some(window) = crate::ui::window() {
        f(&window);
    }
}

/// Applies the settings to the theme and the settings panel.
fn show(window: &AppWindow, settings: &Settings, cover: Option<CoverColors>) {
    apply(window, settings, cover);
    // The built-in palettes, then the custom one with its current colours.
    let palettes: Vec<PaletteChoice> = PALETTES
        .iter()
        .copied()
        .chain([settings.custom_palette.palette()])
        .map(|p| {
            let [bg, fg, accent] = p.swatch();
            PaletteChoice {
                name: p.name.into(),
                bg,
                fg,
                accent,
            }
        })
        .collect();
    window.set_palettes(ModelRc::from(Rc::new(VecModel::from(palettes))));
    let index = if settings.palette == CUSTOM {
        PALETTES.len()
    } else {
        PALETTES.iter().position(|p| p.id == settings.palette).unwrap_or(0)
    };
    window.set_palette(index as i32);
    let custom = &settings.custom_palette;
    window.set_custom_bg(custom.bg.as_str().into());
    window.set_custom_fg(custom.fg.as_str().into());
    window.set_custom_palette_accent(custom.accent.as_str().into());
    let accent = match settings.accent.as_deref() {
        None => -1,
        Some(accent) => ACCENTS
            .iter()
            .position(|a| a.eq_ignore_ascii_case(accent))
            .map_or(-2, |i| i as i32),
    };
    window.set_accent(accent);
    window.set_custom_accent(if accent == -2 { settings.accent.clone().unwrap_or_default() } else { String::new() }.into());
    window.set_cover_colors(settings.cover_colors);
    window.set_font_size_index(FontSize::ALL.iter().position(|f| *f == settings.font_size).unwrap_or(1) as i32);
    window.set_side_visible(settings.side_panel != SidePanel::Hidden);
    window.set_side_tab(match settings.side_panel {
        SidePanel::Queue => SideTab::Queue,
        _ => SideTab::Lyrics,
    });
}
