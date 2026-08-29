use crate::components::game::live_game::GameState;
use crate::components::game::matchup::BatterSide;
use crate::components::game::strikezone::{
    DEFAULT_SZ_BOT, DEFAULT_SZ_TOP, HOME_PLATE_WIDTH, StrikeZone,
};
use crate::ui::gameday::pitch_tracker::{PitchTrackerWidget, TRACKER_WIDTH, fits_beside_zone};
use crate::ui::styling::{border_style, contrast_on, dim_style};
use tui::prelude::*;
use tui::widgets::canvas::{Canvas, Rectangle};
use tui::widgets::{Block, Borders, Paragraph, Wrap};

pub struct AtBatWidget<'a> {
    pub game: &'a GameState,
    pub selected_at_bat: Option<u8>,
    pub show_pitch_tracker: bool,
}

impl Widget for AtBatWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (at_bat, _is_current) = self
            .game
            .get_at_bat_by_index_or_current(self.selected_at_bat);
        let pitches = &at_bat.pitches;

        let [kzone, hit, pitch_info] = Layout::vertical([
            Constraint::Percentage(65), // heatmap/pitches
            Constraint::Length(1),      // hit stats
            Constraint::Percentage(35), // pitch info
        ])
        .horizontal_margin(2)
        .vertical_margin(1)
        .areas(area);

        // The tracker takes a fixed column off the right of the zone row, but only
        // when what is left still renders a zone worth looking at. Below that the
        // zone keeps the whole row — a squashed heat map beside a stat table is
        // worse than either one alone.
        let show_tracker = self.show_pitch_tracker && fits_beside_zone(kzone.width);
        let (kzone, tracker_area) = if show_tracker {
            let [zone, tracker] =
                Layout::horizontal([Constraint::Fill(1), Constraint::Length(TRACKER_WIDTH)])
                    .areas(kzone);
            (zone, Some(tracker))
        } else {
            (kzone, None)
        };

        // grab the strike zone from the first pitch since it doesn't change during the at bat.
        let mut strike_zone_bot = DEFAULT_SZ_BOT * 12.0;
        let mut strike_zone_top = DEFAULT_SZ_TOP * 12.0;
        for pe in &pitches.pitches.pitch_events {
            if let Some(pitch) = pe.pitch.as_ref() {
                strike_zone_bot = pitch.strike_zone_bot * 12.0;
                strike_zone_top = pitch.strike_zone_top * 12.0;
                break;
            }
        }
        let height = strike_zone_top - strike_zone_bot;
        let coords = StrikeZone::build_coords(strike_zone_bot, strike_zone_top);
        let strike_zone_area = generate_strike_zone_area(kzone);
        let pitch_locations = pitches
            .pitches
            .pitch_events
            .iter()
            .filter_map(|event| event.pitch.as_ref().map(|pitch| pitch.plot_position()));
        let (x_bounds, y_bounds) =
            strike_zone_canvas_bounds(strike_zone_bot, strike_zone_top, pitch_locations);

        // A marker wider than one cell has to be nudged left to stay centred on the
        // pitch, and the canvas only knows inches, so measure a cell here.
        let cell_width = (x_bounds[1] - x_bounds[0]) / strike_zone_area.width.max(1) as f64;

        // strike zone and pitch display
        Canvas::default()
            .block(Block::default().borders(Borders::NONE))
            .paint(|ctx| {
                for (i, coord) in coords.iter().enumerate() {
                    let r = Rectangle {
                        x: coord.0,
                        y: coord.1,
                        width: (HOME_PLATE_WIDTH / 3.0),
                        height: (height / 3.0),
                        color: pitches.strike_zone.colors[i],
                    };
                    ctx.draw(&r);
                }
                for pe in &pitches.pitches.pitch_events {
                    if let Some(pitch) = &pe.pitch {
                        let (x, y) = pitch.plot_position();
                        let label = pitch.index.to_string();
                        ctx.print(
                            label_origin(x, label.chars().count(), cell_width),
                            y,
                            Span::styled(label, marker_style(pitch.color)),
                        );
                    }
                }
            })
            .x_bounds(x_bounds)
            .y_bounds(y_bounds)
            .render(strike_zone_area, buf);

        render_batter_side(
            at_bat.matchup.bat_side,
            self.game
                .players
                .get(&at_bat.matchup.batter_id)
                .map(|p| p.last_name.as_str())
                .unwrap_or_default(),
            strike_zone_area,
            buf,
        );

        if let Some(tracker_area) = tracker_area {
            PitchTrackerWidget {
                game: self.game,
                at_bat_index: at_bat.index,
            }
            .render(tracker_area, buf);
        }

        // display the event information
        let events: Vec<Line> = pitches
            .pitches
            .pitch_events
            .iter()
            .rev() // reverse so that the last event is at the top
            .filter_map(|event| {
                event.as_lines(
                    false,
                    &self.game.home_team,
                    &self.game.away_team,
                    &self.game.players,
                )
            })
            .flatten()
            .collect();

        let paragraph = Paragraph::new(events).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(border_style()),
        );
        Widget::render(paragraph, pitch_info, buf);

        // display the hit information if available
        if let Some(data) = pitches.pitches.pitch_events.last()
            && let Some(text) = data.format_hit_data()
        {
            let paragraph = Paragraph::new(text)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false });
            Widget::render(paragraph, hit, buf);
        };
    }
}

/// The pitch number *is* the marker, printed in reverse video: the pitch's own
/// color as the background with black or white text over it, bold.
///
/// It was a 1 inch `Rectangle`, which rendered as four faint braille dots in a
/// square, and briefly a braille crosshair, which was worse — both drew a shape
/// around the location instead of marking it, and neither survived being laid over
/// a colored heat map. A filled chip is unmistakable against any zone color and
/// carries the pitch number in the same character cells.
fn marker_style(color: Color) -> Style {
    Style::default()
        .fg(contrast_on(color))
        .bg(color)
        .add_modifier(Modifier::BOLD)
}

/// Where to start printing a marker so it stays centred on the pitch.
///
/// `Context::print` puts the label's *left edge* at the given x, so a two digit
/// pitch number would sit half a cell right of where the pitch actually crossed.
/// Shifting left by half a cell per extra character straddles the true location.
fn label_origin(x: f64, label_len: usize, cell_width: f64) -> f64 {
    x - cell_width * (label_len.saturating_sub(1) as f64) / 2.0
}

/// Which column of the zone area the batter's label belongs in.
///
/// The chart is drawn in raw `pX`, which the API reports from the catcher's
/// perspective: a right handed batter stands in the third base box, which is to the
/// catcher's left, so a righty belongs on the **left** of the chart and a lefty on
/// the right.
///
/// That is measured, not reasoned. Hit-by-pitches settle it, because a pitch that
/// hits the batter is on the batter's own side of the plate: across every HBP in the
/// 62 final games of 2026-08-24 through 08-28, all 35 to right handed batters had
/// negative `pX` and all 27 to left handed batters had positive `pX`. No exceptions.
/// If this ever needs flipping, it is the one place to do it.
fn label_column(side: BatterSide, area: Rect) -> Option<u16> {
    if area.width == 0 {
        return None;
    }
    match side {
        BatterSide::Right => Some(area.x),
        BatterSide::Left => Some(area.x + area.width - 1),
        BatterSide::Unknown => None,
    }
}

/// The batter's surname printed one character per row down the side of the zone he
/// stands on, falling back to `RHB`/`LHB` when the panel is too short for the name.
fn batter_label(side: BatterSide, last_name: &str, available_rows: u16) -> Option<String> {
    let short = side.label()?;
    let name: String = last_name
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let name = name.to_uppercase();

    if !name.is_empty() && name.chars().count() <= available_rows as usize {
        Some(name)
    } else if short.len() <= available_rows as usize {
        Some(short.to_string())
    } else {
        None
    }
}

fn render_batter_side(side: BatterSide, last_name: &str, area: Rect, buf: &mut Buffer) {
    let Some(x) = label_column(side, area) else {
        return;
    };
    let Some(text) = batter_label(side, last_name, area.height) else {
        return;
    };

    let rows = text.chars().count() as u16;
    let y = area.y + area.height.saturating_sub(rows) / 2;
    let lines: Vec<Line> = text
        .chars()
        .map(|c| Line::from(Span::styled(c.to_string(), dim_style())))
        .collect();

    Paragraph::new(lines).render(Rect::new(x, y, 1, rows), buf);
}

fn strike_zone_canvas_bounds(
    strike_zone_bot: f64,
    strike_zone_top: f64,
    pitch_locations: impl IntoIterator<Item = (f64, f64)>,
) -> ([f64; 2], [f64; 2]) {
    const HORIZONTAL_MARGIN: f64 = 6.0;
    const VERTICAL_MARGIN: f64 = 8.0;
    const PITCH_MARKER_MARGIN: f64 = 2.0;

    let mut x_bounds = [
        -(HOME_PLATE_WIDTH / 2.0) - HORIZONTAL_MARGIN,
        HOME_PLATE_WIDTH / 2.0 + HORIZONTAL_MARGIN,
    ];
    let mut y_bounds = [
        strike_zone_bot - VERTICAL_MARGIN,
        strike_zone_top + VERTICAL_MARGIN,
    ];

    for (x, y) in pitch_locations {
        x_bounds[0] = x_bounds[0].min(x - PITCH_MARKER_MARGIN);
        x_bounds[1] = x_bounds[1].max(x + PITCH_MARKER_MARGIN);
        y_bounds[0] = y_bounds[0].min(y - PITCH_MARKER_MARGIN);
        y_bounds[1] = y_bounds[1].max(y + PITCH_MARKER_MARGIN);
    }

    (x_bounds, y_bounds)
}

fn generate_strike_zone_area(area: Rect) -> Rect {
    // kind of arbitrary values to make the zone look good
    const STRIKE_ZONE_WIDTH: u16 = 35;
    const STRIKE_ZONE_HEIGHT: u16 = 19;
    let desired_aspect_ratio = STRIKE_ZONE_WIDTH as f64 / STRIKE_ZONE_HEIGHT as f64;

    let available_width = area.width;
    let available_height = area.height;

    // calculate what dimensions would fit while maintaining aspect ratio
    let width_constrained_height = (available_width as f64 / desired_aspect_ratio) as u16;
    let height_constrained_width = (available_height as f64 * desired_aspect_ratio) as u16;

    let (final_width, final_height) = if width_constrained_height <= available_height {
        // width is the limiting factor
        (available_width, width_constrained_height)
    } else {
        // height is the limiting factor
        (height_constrained_width, available_height)
    };

    // center the calculated dimensions
    let x_offset = (available_width - final_width) / 2;
    let y_offset = (available_height - final_height) / 2;

    Rect::new(
        area.x + x_offset,
        area.y + y_offset,
        final_width,
        final_height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strike_zone_bounds_keep_a_compact_visible_margin() {
        let (x, y) = strike_zone_canvas_bounds(18.0, 39.6, []);

        assert_eq!(x, [-14.5, 14.5]);
        assert_eq!(y, [10.0, 47.6]);
    }

    #[test]
    fn strike_zone_bounds_expand_for_wild_pitches() {
        let (x, y) = strike_zone_canvas_bounds(18.0, 39.6, [(-20.0, 50.0)]);

        assert_eq!(x, [-22.0, 14.5]);
        assert_eq!(y, [10.0, 52.0]);
    }

    #[test]
    fn a_marker_is_the_pitch_colour_filled_in() {
        // reverse video: the pitch colour becomes the background so the chip reads
        // against any heat map zone underneath it
        let style = marker_style(Color::Rgb(170, 21, 11));
        assert_eq!(style.bg, Some(Color::Rgb(170, 21, 11)));
        assert_eq!(style.fg, Some(Color::White));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_marker_on_a_bright_pitch_colour_flips_to_black_text() {
        assert_eq!(
            marker_style(Color::Rgb(255, 255, 255)).fg,
            Some(Color::Black)
        );
    }

    #[test]
    fn a_single_digit_marker_sits_on_the_pitch() {
        assert_eq!(label_origin(4.0, 1, 0.83), 4.0);
    }

    #[test]
    fn a_two_digit_marker_straddles_the_pitch() {
        // print() anchors the left edge, so pitch 10 would otherwise sit half a cell
        // right of where it actually crossed
        assert_eq!(label_origin(4.0, 2, 0.8), 3.6);
        assert_eq!(label_origin(4.0, 3, 0.8), 3.2);
    }

    #[test]
    fn a_righty_is_labelled_on_the_third_base_side() {
        // verified against every HBP in 62 games (2026-08-24..28): 35/35 to righties
        // had negative pX, 27/27 to lefties positive. See `label_column`.
        let area = Rect::new(10, 0, 20, 12);
        assert_eq!(label_column(BatterSide::Right, area), Some(10));
        assert_eq!(label_column(BatterSide::Left, area), Some(29));
        assert_eq!(label_column(BatterSide::Unknown, area), None);
    }

    #[test]
    fn the_batter_label_prefers_the_name() {
        assert_eq!(
            batter_label(BatterSide::Right, "Devers", 12).as_deref(),
            Some("DEVERS")
        );
    }

    #[test]
    fn a_name_too_tall_for_the_panel_falls_back_to_the_side() {
        assert_eq!(
            batter_label(BatterSide::Left, "Featherstonhaugh", 8).as_deref(),
            Some("LHB")
        );
        assert_eq!(
            batter_label(BatterSide::Right, "Devers", 4).as_deref(),
            Some("RHB")
        );
    }

    #[test]
    fn a_panel_too_short_for_either_shows_nothing() {
        assert_eq!(batter_label(BatterSide::Right, "Devers", 2), None);
    }

    #[test]
    fn a_missing_name_still_labels_the_side() {
        assert_eq!(
            batter_label(BatterSide::Right, "", 12).as_deref(),
            Some("RHB")
        );
    }

    #[test]
    fn an_unknown_side_is_never_labelled() {
        // a switch hitter before the play resolves, or a game with no matchup yet
        assert_eq!(batter_label(BatterSide::Unknown, "Devers", 12), None);
    }

    #[test]
    fn a_two_part_surname_stays_on_one_column() {
        assert_eq!(
            batter_label(BatterSide::Right, "De La Cruz", 12).as_deref(),
            Some("DELACRUZ")
        );
    }
}
