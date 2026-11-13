//! The app's look: Inter, a warm neutral palette with a teal accent, and
//! roomier spacing than egui's defaults, in light and dark variants that
//! follow the system setting.
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontFamily, FontId, Margin, Stroke, Style, TextStyle,
    Theme, Visuals,
    epaint::text::{FontInsert, FontPriority, InsertFontFamily},
    vec2,
};

static INTER: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
static INTER_SEMIBOLD: &[u8] = include_bytes!("../fonts/Inter-SemiBold.ttf");

pub struct Palette {
    /// Window and panel background.
    pub background: Color32,
    /// Cards and text fields.
    pub surface: Color32,
    /// Buttons and other controls at rest.
    pub raised: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Text drawn on `accent`.
    pub on_accent: Color32,
    pub error: Color32,
    pub success: Color32,
}

pub const LIGHT: Palette = Palette {
    background: Color32::from_rgb(0xF4, 0xF2, 0xEE),
    surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    raised: Color32::from_rgb(0xEC, 0xE9, 0xE3),
    border: Color32::from_rgb(0xDA, 0xD5, 0xCC),
    text: Color32::from_rgb(0x1E, 0x1C, 0x19),
    muted: Color32::from_rgb(0x66, 0x61, 0x5A),
    accent: Color32::from_rgb(0x0F, 0x6E, 0x66),
    accent_hover: Color32::from_rgb(0x0B, 0x57, 0x51),
    on_accent: Color32::WHITE,
    error: Color32::from_rgb(0xB4, 0x23, 0x18),
    success: Color32::from_rgb(0x1F, 0x6F, 0x3A),
};

pub const DARK: Palette = Palette {
    background: Color32::from_rgb(0x17, 0x16, 0x14),
    surface: Color32::from_rgb(0x22, 0x20, 0x1D),
    raised: Color32::from_rgb(0x2D, 0x2A, 0x26),
    border: Color32::from_rgb(0x3D, 0x39, 0x34),
    text: Color32::from_rgb(0xEE, 0xEB, 0xE5),
    muted: Color32::from_rgb(0xA8, 0xA2, 0x98),
    accent: Color32::from_rgb(0x5E, 0xC8, 0xB8),
    accent_hover: Color32::from_rgb(0x7F, 0xD6, 0xC8),
    on_accent: Color32::from_rgb(0x0A, 0x22, 0x1F),
    error: Color32::from_rgb(0xF2, 0x8B, 0x82),
    success: Color32::from_rgb(0x7F, 0xD1, 0x95),
};

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn install(ctx: &egui::Context) {
    ctx.add_font(FontInsert::new(
        "inter",
        FontData::from_static(INTER),
        vec![InsertFontFamily {
            family: FontFamily::Proportional,
            priority: FontPriority::Highest,
        }],
    ));
    ctx.add_font(FontInsert::new(
        "inter-semibold",
        FontData::from_static(INTER_SEMIBOLD),
        vec![InsertFontFamily {
            family: semibold(),
            priority: FontPriority::Highest,
        }],
    ));
    ctx.set_style_of(Theme::Light, style(&LIGHT, Visuals::light()));
    ctx.set_style_of(Theme::Dark, style(&DARK, Visuals::dark()));
}

fn style(p: &Palette, mut visuals: Visuals) -> Style {
    let mut style = Style {
        text_styles: [
            (TextStyle::Heading, FontId::new(20.0, semibold())),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Small, FontId::proportional(12.5)),
            (TextStyle::Monospace, FontId::monospace(13.5)),
        ]
        .into(),
        ..Default::default()
    };
    style.spacing.item_spacing = vec2(10.0, 10.0);
    style.spacing.button_padding = vec2(14.0, 7.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.window_margin = Margin::same(16);
    style.spacing.menu_margin = Margin::same(8);

    let radius = CornerRadius::same(8);
    visuals.panel_fill = p.background;
    visuals.window_fill = p.surface;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.extreme_bg_color = p.surface;
    visuals.faint_bg_color = p.raised;
    visuals.code_bg_color = p.raised;
    visuals.hyperlink_color = p.accent;
    visuals.error_fg_color = p.error;
    visuals.warn_fg_color = p.error;
    visuals.selection.bg_fill = selection(p);
    visuals.selection.stroke = Stroke::new(1.0, p.text);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = p.surface;
    widgets.noninteractive.weak_bg_fill = p.surface;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.inactive.bg_fill = p.raised;
    widgets.inactive.weak_bg_fill = p.raised;
    widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.hovered.bg_fill = p.raised;
    widgets.hovered.weak_bg_fill = p.raised;
    widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    // egui grows hovered widgets by default, which makes rows jitter.
    widgets.hovered.expansion = 0.0;
    widgets.active.bg_fill = p.border;
    widgets.active.weak_bg_fill = p.border;
    widgets.active.bg_stroke = Stroke::new(1.5, p.accent);
    widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    widgets.active.expansion = 0.0;
    widgets.open = widgets.active;
    for state in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.corner_radius = radius;
    }

    style.visuals = visuals;
    style
}

/// The translucent fill behind selected text, tabs and toggles.
fn selection(p: &Palette) -> Color32 {
    p.accent.gamma_multiply(0.35)
}

pub fn palette(ui: &egui::Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

const CARD_PADDING: i8 = 16;
const CARD_STROKE: f32 = 1.0;

/// Width a card adds around its contents: padding and border on both sides.
pub const CARD_CHROME: f32 = 2.0 * (CARD_PADDING as f32 + CARD_STROKE);

pub fn card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let p = palette(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(CARD_STROKE, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(CARD_PADDING))
        .show(ui, add_contents)
        .inner
}

pub fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let p = palette(ui);
    ui.scope(|ui| {
        // Colouring through the widget visuals, rather than fixed fill and
        // text colours, lets hover and press change the fill alone. A
        // disabled button may be styled as non-interactive; it keeps the
        // accent and egui fades it.
        let widgets = &mut ui.visuals_mut().widgets;
        for (state, fill) in [
            (&mut widgets.noninteractive, p.accent),
            (&mut widgets.inactive, p.accent),
            (&mut widgets.hovered, p.accent_hover),
            (&mut widgets.active, p.accent_hover),
        ] {
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.0, p.on_accent);
        }
        let label = egui::RichText::new(text).font(FontId::new(16.0, semibold()));
        let button = egui::Button::new(label)
            .corner_radius(CornerRadius::same(8))
            .min_size(vec2(140.0, 40.0));
        ui.add_enabled(enabled, button)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    })
    .inner
}

/// WCAG 2 contrast ratio between two opaque colours.
#[cfg(test)]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn luminance(c: Color32) -> f32 {
        let channel = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `over`, which may be translucent (premultiplied, as egui stores it),
/// composited onto opaque `under`.
#[cfg(test)]
fn composite(over: Color32, under: Color32) -> Color32 {
    let keep = 1.0 - f32::from(over.a()) / 255.0;
    let mix = |o: u8, u: u8| (f32::from(o) + f32::from(u) * keep).round() as u8;
    Color32::from_rgb(
        mix(over.r(), under.r()),
        mix(over.g(), under.g()),
        mix(over.b(), under.b()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_pairing_meets_wcag_aa() {
        for (name, p) in [("light", &LIGHT), ("dark", &DARK)] {
            for (what, fg, bg) in [
                ("text on background", p.text, p.background),
                ("text on surface", p.text, p.surface),
                ("text on raised", p.text, p.raised),
                ("muted on background", p.muted, p.background),
                ("muted on surface", p.muted, p.surface),
                ("accent on surface", p.accent, p.surface),
                ("on_accent on accent", p.on_accent, p.accent),
                ("on_accent on accent_hover", p.on_accent, p.accent_hover),
                ("error on surface", p.error, p.surface),
                ("success on surface", p.success, p.surface),
                ("error on background", p.error, p.background),
                ("success on background", p.success, p.background),
                ("muted on raised", p.muted, p.raised),
                // Selected tabs and toggles sit on the panel and on cards.
                (
                    "text on selection over background",
                    p.text,
                    composite(selection(p), p.background),
                ),
                (
                    "text on selection over surface",
                    p.text,
                    composite(selection(p), p.surface),
                ),
            ] {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 4.5, "{name}: {what} is {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn contrast_matches_known_values() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.01);
    }
}
