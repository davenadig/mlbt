use log::error;
use tui::prelude::Color;
use tui::style::Style;

/// Default text color which allows the terminal theme to supply the color.
pub const TEXT_COLOR: Color = Color::Reset;

/// Default border color which allows the terminal theme to supply the color.
const BORDER_COLOR: Color = Color::Reset;

const UNDERLINE_COLOR: Color = Color::Reset;

pub fn border_style() -> Style {
    Style::new().fg(BORDER_COLOR)
}

pub fn header_style() -> Style {
    Style::new()
        .bold()
        .underlined()
        .underline_color(UNDERLINE_COLOR)
}

pub fn text_style() -> Style {
    Style::new().fg(TEXT_COLOR)
}

pub fn dim_style() -> Style {
    text_style().dim()
}

pub fn selected_style() -> Style {
    Style::new().fg(Color::Black).bg(Color::Blue)
}

/// Returns a `Style` with `TEXT_COLOR` modified with `Modifier::DIM` if the value is zero,
/// otherwise a default style with `TEXT_COLOR`.
pub trait DimStyle {
    fn is_zero(&self) -> bool;

    fn dim_or_default(&self) -> Style {
        if self.is_zero() {
            dim_style()
        } else {
            text_style()
        }
    }
}

macro_rules! impl_is_zero {
    ($($t:ty),*) => {
        $(impl DimStyle for $t {
            fn is_zero(&self) -> bool { *self == 0 }
        })*
    };
}
impl_is_zero!(u8, u16);

impl DimStyle for str {
    fn is_zero(&self) -> bool {
        // TODO should this check floats too, e.g. "0.0"?
        self == "0"
    }
}

/// Color for an ERA stat string
pub fn era_color(era: &str) -> Color {
    era.parse::<f64>()
        .map(|v| {
            if v <= 3.00 {
                Color::Green
            } else if v >= 5.00 {
                Color::Red
            } else {
                TEXT_COLOR
            }
        })
        .unwrap_or(TEXT_COLOR)
}

/// Style for an ERA stat string.
pub fn era_style(era: &str) -> Style {
    Style::new().fg(era_color(era))
}

/// Color for a batting average stat string.
pub fn avg_color(avg: &str) -> Color {
    avg.parse::<f64>()
        .map(|v| {
            if v == 0.0 {
                TEXT_COLOR
            } else if v >= 0.300 {
                Color::Green
            } else if v < 0.100 {
                Color::Red
            } else {
                TEXT_COLOR
            }
        })
        .unwrap_or(TEXT_COLOR)
}

/// Style for a batting average stat string.
pub fn avg_style(avg: &str) -> Style {
    Style::new().fg(avg_color(avg))
}

/// Color for a winning-percentage stat string.
pub fn win_pct_color(pct: &str) -> Color {
    pct.parse::<f64>()
        .map(|v| {
            if v == 0.0 {
                TEXT_COLOR
            } else if v >= 0.500 {
                Color::Green
            } else {
                Color::Red
            }
        })
        .unwrap_or(TEXT_COLOR)
}

/// Pick black or white — whichever has more contrast printed on `background`.
///
/// The API supplies pitch colors and they run from a dark red called strike to a
/// bright green ball, so no fixed foreground serves all of them.
///
/// This uses the WCAG relative luminance rather than a plain luma average, and the
/// difference is not academic: a mid green like `rgb(0, 200, 0)` reads as *dark*
/// under Rec. 601 luma and would get white text, which lands at a 2.2:1 contrast
/// ratio. Black on the same green is 9.4:1.
pub fn contrast_on(background: Color) -> Color {
    match background {
        Color::Rgb(r, g, b) => {
            let luminance = relative_luminance(r, g, b);
            // the ratio against white is 1.05/(L+0.05) and against black is
            // (L+0.05)/0.05; they cross at L = 0.179
            if luminance > 0.179 {
                Color::Black
            } else {
                Color::White
            }
        }
        // a named or indexed color's brightness is a terminal theme decision, so
        // there is nothing to measure; white on the usual dark palettes is safer
        _ => Color::White,
    }
}

/// WCAG relative luminance of an sRGB color, 0.0 (black) to 1.0 (white).
fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    fn linearize(channel: u8) -> f64 {
        let c = channel as f64 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b)
}

/// Convert a string from the API to a Color::Rgb. The string starts out as:
/// "rgba(255, 255, 255, 0.55)".
pub fn convert_color(s: String) -> Color {
    if let Some(s) = s.strip_prefix("rgba(") {
        let c: Vec<&str> = s.split(", ").collect();
        Color::Rgb(
            c[0].parse().unwrap_or(0),
            c[1].parse().unwrap_or(0),
            c[2].parse().unwrap_or(0),
        )
    } else {
        error!("color doesn't start with 'rgba(' {s:?}");
        Color::Rgb(0, 0, 0)
    }
}

/// WCAG contrast ratio between two luminances, lighter first.
#[cfg(test)]
fn contrast_ratio(a: f64, b: f64) -> f64 {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn contrast_anchors_at_the_extremes() {
    assert_eq!(contrast_on(Color::Rgb(255, 255, 255)), Color::Black);
    assert_eq!(contrast_on(Color::Rgb(0, 0, 0)), Color::White);
}

#[test]
fn contrast_always_picks_the_more_readable_of_the_two() {
    // asserting the property rather than a table of expected colors: the point is
    // that the chosen foreground wins on contrast, not that it is any given colour
    let samples = [
        (170, 21, 11), // called strike red
        (39, 161, 39), // ball green
        (0, 200, 0),   // the mid green a luma average gets wrong
        (0, 0, 200),
        (150, 188, 255), // in play, light blue
        (6, 90, 238),
        (255, 255, 0),
        (128, 128, 128),
    ];
    for (r, g, b) in samples {
        let background = relative_luminance(r, g, b);
        let against_white = contrast_ratio(1.0, background);
        let against_black = contrast_ratio(0.0, background);
        let expected = if against_black > against_white {
            Color::Black
        } else {
            Color::White
        };
        assert_eq!(
            contrast_on(Color::Rgb(r, g, b)),
            expected,
            "rgb({r}, {g}, {b}) picked the lower contrast foreground"
        );
    }
}

#[test]
fn a_themed_color_defaults_to_white() {
    assert_eq!(contrast_on(Color::Reset), Color::White);
}

#[test]
fn test_color_conversion() {
    let tests = vec![
        ("rgba(0, 0, 0, .55)", Color::Rgb(0, 0, 0)),
        ("rgba(6, 90, 238, .55)", Color::Rgb(6, 90, 238)),
        ("rgba(150, 188, 255, .55)", Color::Rgb(150, 188, 255)),
        ("rgba(214, 41, 52, .55)", Color::Rgb(214, 41, 52)),
        ("rgba(255, 255, 255, 0.55)", Color::Rgb(255, 255, 255)),
    ];
    for t in tests {
        assert_eq!(convert_color(t.0.to_string()), t.1);
    }

    let bad = ("rgba(55, 255, 255, 0.55)", Color::Rgb(255, 255, 255));
    assert_ne!(convert_color(bad.0.to_string()), bad.1);

    let nonsense = ("rgba(-5, 255, 255, 0.55)", Color::Rgb(0, 255, 255));
    assert_eq!(convert_color(nonsense.0.to_string()), nonsense.1);
}
