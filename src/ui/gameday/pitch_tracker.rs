//! The pitch tracker column that sits beside the strike zone.
//!
//! Shows what the pitcher of the viewed at bat has thrown so far in this game:
//! total pitches, strike rate, and a row per pitch type with its share and
//! velocity. Aggregation lives in `components::game::pitch_tracker`; this only
//! formats it to whatever width the column got.

use crate::components::game::live_game::GameState;
use crate::components::game::pitch_tracker::{PitchTypeTotals, PitcherTotals};
use crate::ui::styling::{dim_style, text_style};
use tui::prelude::*;
use tui::widgets::{Block, Borders, Paragraph};

/// Width of the tracker column. Sized to the widest row it prints — a two letter
/// code, a three digit count, a share and a velocity — plus one space of gutter.
pub const TRACKER_WIDTH: u16 = 16;

/// Below this the strike zone stops being worth looking at, so the tracker gives up
/// its column rather than squeezing both. The zone's own aspect ratio is 35x19.
pub const MIN_ZONE_WIDTH: u16 = 22;

/// Whether there is room to show the zone and the tracker side by side.
pub fn fits_beside_zone(available_width: u16) -> bool {
    available_width >= MIN_ZONE_WIDTH + TRACKER_WIDTH
}

pub struct PitchTrackerWidget<'a> {
    pub game: &'a GameState,
    pub at_bat_index: u8,
}

impl Widget for PitchTrackerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Some(totals) = self.game.pitcher_totals_through(self.at_bat_index) else {
            return;
        };

        let name = self
            .game
            .players
            .get(&totals.pitcher_id)
            .map(|p| p.last_name.clone())
            .unwrap_or_default();

        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(dim_style());
        let inner = block.inner(area);
        block.render(area, buf);

        Paragraph::new(lines(&name, &totals, inner.width, inner.height)).render(inner, buf);
    }
}

/// Build the column's lines for the given inner width and height.
///
/// Split out from `render` so the formatting is testable without a terminal — the
/// alignment is the whole point of the column and it is easy to break silently.
fn lines<'a>(name: &str, totals: &PitcherTotals, width: u16, height: u16) -> Vec<Line<'a>> {
    let width = width as usize;
    let mut lines = vec![
        Line::from(Span::styled(truncate(name, width), text_style().bold())),
        Line::from(Span::styled(summary(totals), text_style())),
    ];

    if totals.pitches == 0 {
        return lines;
    }

    lines.push(Line::from(Span::styled("─".repeat(width), dim_style())));

    // Whatever rows are left after the name, summary and rule. A pitcher with more
    // types than fit loses the least thrown ones, which is the right end to cut.
    let room = (height as usize).saturating_sub(lines.len());
    for pitch_type in totals.types.iter().take(room) {
        lines.push(Line::from(Span::styled(
            type_row(pitch_type, width),
            text_style(),
        )));
    }
    lines
}

/// `108P 72S 67%` — pitches, strikes, strike rate.
fn summary(totals: &PitcherTotals) -> String {
    format!(
        "{}P {}S {:.0}%",
        totals.pitches,
        totals.strikes,
        totals.strike_share()
    )
}

/// `FF  28 41%  97` — code, count, share, average velocity. Max velocity is added
/// when the column is wide enough for it, which it is not at the default width.
fn type_row(pitch_type: &PitchTypeTotals, width: usize) -> String {
    let velo = format_speed(pitch_type.avg_speed);
    let base = format!(
        "{:<2} {:>3} {:>2.0}% {:>3}",
        truncate(&pitch_type.code, 2),
        pitch_type.count,
        pitch_type.share,
        velo,
    );
    // 4 more columns buys the top end, which is what tells you whether he is still
    // reaching back for it late in the outing
    if width >= base.chars().count() + 4 {
        format!("{base} {:>3}", format_speed(pitch_type.max_speed))
    } else {
        base
    }
}

/// Velocities round to whole mph — the column has no room for a decimal, and the
/// difference between 96.4 and 96 is not what anyone reads this for. An untracked
/// pitch reports 0.0 and prints as a dash rather than a fake `0`.
fn format_speed(speed: f64) -> String {
    if speed <= 0.0 {
        "-".to_string()
    } else {
        format!("{speed:.0}")
    }
}

fn truncate(s: &str, width: usize) -> String {
    s.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn totals(pitches: usize, strikes: usize, types: Vec<PitchTypeTotals>) -> PitcherTotals {
        PitcherTotals {
            pitcher_id: 1,
            pitches,
            strikes,
            types,
        }
    }

    fn pitch_type(code: &str, count: usize, share: f64, avg: f64, max: f64) -> PitchTypeTotals {
        PitchTypeTotals {
            code: code.to_string(),
            count,
            share,
            avg_speed: avg,
            max_speed: max,
        }
    }

    #[test]
    fn a_type_row_fits_the_default_column() {
        let row = type_row(
            &pitch_type("FF", 28, 41.0, 96.6, 99.1),
            TRACKER_WIDTH as usize,
        );
        assert_eq!(row, "FF  28 41%  97");
        assert!(row.chars().count() <= TRACKER_WIDTH as usize);
    }

    #[test]
    fn a_wide_column_gains_the_top_velocity() {
        let row = type_row(&pitch_type("FF", 28, 41.0, 96.6, 99.1), 24);
        assert!(row.ends_with(" 99"), "got {row:?}");
    }

    #[test]
    fn a_three_digit_count_does_not_shift_the_columns() {
        // a starter can throw 100+ of one pitch; the count column is sized for it
        let narrow = type_row(
            &pitch_type("FF", 9, 41.0, 96.6, 99.1),
            TRACKER_WIDTH as usize,
        );
        let wide = type_row(
            &pitch_type("FF", 128, 41.0, 96.6, 99.1),
            TRACKER_WIDTH as usize,
        );
        assert_eq!(narrow.chars().count(), wide.chars().count());
    }

    #[test]
    fn an_untracked_velocity_prints_a_dash_not_a_zero() {
        let row = type_row(&pitch_type("PO", 2, 3.0, 0.0, 0.0), TRACKER_WIDTH as usize);
        assert!(row.ends_with('-'), "got {row:?}");
    }

    #[test]
    fn summary_reads_pitches_strikes_rate() {
        assert_eq!(summary(&totals(68, 44, vec![])), "68P 44S 65%");
    }

    #[test]
    fn a_pitcher_with_no_pitches_shows_his_name_and_stops() {
        let rendered = lines("Cole", &totals(0, 0, vec![]), TRACKER_WIDTH, 10);
        assert_eq!(rendered.len(), 2, "name and summary, no rule and no rows");
        assert_eq!(rendered[1].to_string(), "0P 0S 0%");
    }

    #[test]
    fn rows_are_dropped_from_the_least_thrown_end_when_space_runs_out() {
        let t = totals(
            60,
            40,
            vec![
                pitch_type("FF", 30, 50.0, 96.0, 99.0),
                pitch_type("SL", 20, 33.0, 87.0, 89.0),
                pitch_type("CH", 10, 17.0, 88.0, 90.0),
            ],
        );
        // 3 header lines + 1 row of room
        let rendered = lines("Cole", &t, TRACKER_WIDTH, 4);
        assert_eq!(rendered.len(), 4);
        assert!(rendered[3].to_string().starts_with("FF"));
    }

    #[test]
    fn a_long_name_is_cut_to_the_column() {
        let rendered = lines("Featherstonhaugh", &totals(1, 1, vec![]), 8, 10);
        assert_eq!(rendered[0].to_string().chars().count(), 8);
    }

    #[test]
    fn the_tracker_yields_the_column_when_the_zone_would_be_unreadable() {
        assert!(!fits_beside_zone(MIN_ZONE_WIDTH + TRACKER_WIDTH - 1));
        assert!(fits_beside_zone(MIN_ZONE_WIDTH + TRACKER_WIDTH));
    }
}
