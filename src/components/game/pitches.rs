use crate::components::game::live_game::PlayerMap;
use crate::components::game::pitch_event::PitchEvent;
use crate::components::game::review::ReviewDetails;
use crate::components::game::strikezone::{DEFAULT_SZ_BOT, DEFAULT_SZ_TOP};
use crate::components::standings::Team;
use crate::ui::styling::{convert_color, dim_style};
use mlbt_api::plays::{Play, PlayEvent};
use tui::{
    style::{Color, Style},
    text::{Line, Span},
};

#[derive(Debug, Default)]
pub struct Pitches {
    pub pitch_events: Vec<PitchEvent>,
}

/// Spin and break for one pitch, as the API reports it.
///
/// `horizontal` is stored raw, in the API's frame — **the pitcher's perspective**,
/// where positive is a right hander's arm side. The strike zone above it is drawn in
/// `pX`, the catcher's perspective, which is the opposite sign. Rather than pick a
/// convention and hope, the direction is rendered as an arrow pointing the way the
/// ball moved *on that chart*, so the number beside the zone agrees with the picture.
///
/// The relationship is measured: across 6,418 pitches (2026-08-27 and 08-28), right
/// handed pitchers' four-seamers were 97% positive `breakHorizontal` and 98%
/// negative `pfxX`; left handers' were 100% negative and 100% positive. A four-seam
/// fastball runs to the pitcher's arm side, which fixes both frames.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Movement {
    pub spin_rate: Option<u32>,
    /// Induced vertical break in inches: positive rides, negative drops.
    pub induced_vertical: Option<f64>,
    /// Horizontal break in inches, pitcher's perspective (see above).
    pub horizontal: Option<f64>,
}

#[derive(Debug)]
pub struct Pitch {
    pub strike: bool,
    pub color: Color,
    pub description: String, // called strike, hit, strike out, etc.
    pub location: (f64, f64),
    pub index: u8,
    pub pitch_type: String, // fastball, slider, etc.
    /// Short API code for the pitch type, e.g. `FF`, `SL`. The pitch tracker column
    /// is too narrow for the description, and the codes are what a scorer writes.
    pub pitch_type_code: String,
    pub speed: f64,
    pub strike_zone_bot: f64,
    pub strike_zone_top: f64,
    pub count: Count,
    pub review_details: Option<ReviewDetails>,
    pub movement: Movement,
}

impl Default for Pitch {
    fn default() -> Self {
        Pitch {
            strike: false,
            color: Color::Black,
            description: "-".to_string(),
            location: (0.0, 0.0),
            index: 0,
            pitch_type: "-".to_string(),
            pitch_type_code: String::new(),
            speed: 0.0,
            strike_zone_bot: DEFAULT_SZ_BOT,
            strike_zone_top: DEFAULT_SZ_TOP,
            count: Count::default(),
            review_details: None,
            movement: Movement::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Count {
    pub balls: u8,
    pub strikes: u8,
    /// Outs at the time of the pitch. The API reports this *before* the pitch resolves —
    /// a strikeout's third strike still reads the outs it was thrown with — which is
    /// what the scoreboard wants while walking an at bat pitch by pitch.
    pub outs: u8,
}

impl From<mlbt_api::plays::Count> for Count {
    fn from(value: mlbt_api::plays::Count) -> Self {
        Self {
            balls: value.balls,
            strikes: value.strikes,
            outs: value.outs,
        }
    }
}

impl From<&PlayEvent> for Pitch {
    fn from(play: &PlayEvent) -> Self {
        let pitch_data = match play.pitch_data.as_ref() {
            Some(p) => p,
            None => return Pitch::default(),
        };
        let pitch_coords = &pitch_data.coordinates;
        let pitch_details = &play.details;

        // x coordinate is left/right
        // y coordinate is catcher looking towards pitcher
        // z coordinate is up/down
        let x_coord = pitch_coords.get("pX").unwrap_or(&0.0);
        let z_coord = pitch_coords.get("pZ").unwrap_or(&2.0);

        Pitch {
            strike: pitch_details.is_strike.unwrap_or(false),
            speed: pitch_data.start_speed.unwrap_or(0.0),
            color: convert_color(
                pitch_details
                    .ball_color
                    .clone()
                    .unwrap_or_else(|| String::from("rgba(255, 255, 255, 0)")),
            ),
            description: pitch_details.description.clone().unwrap_or_default(),
            pitch_type: pitch_details
                .pitch_type
                .as_ref()
                .map(|pt| pt.description.clone())
                .unwrap_or_default(),
            pitch_type_code: pitch_details
                .pitch_type
                .as_ref()
                .map(|pt| pt.code.clone())
                .unwrap_or_default(),
            location: (*x_coord, *z_coord),
            index: play.pitch_number.unwrap_or_default(),
            strike_zone_bot: pitch_data.strike_zone_bottom.unwrap_or(DEFAULT_SZ_BOT),
            strike_zone_top: pitch_data.strike_zone_top.unwrap_or(DEFAULT_SZ_TOP),
            count: play.count.clone().into(),
            review_details: play.review_details.as_ref().map(ReviewDetails::from),
            movement: pitch_data
                .breaks
                .as_ref()
                .map(|b| Movement {
                    spin_rate: b.spin_rate,
                    induced_vertical: b.break_vertical_induced,
                    horizontal: b.break_horizontal,
                })
                .unwrap_or_default(),
        }
    }
}

impl Movement {
    /// `2144rpm  IVB 13"  HB 5"←` — whatever of the three the API supplied.
    ///
    /// Returns `None` when it supplied none, so a pitch with no tracking data adds
    /// nothing to its line rather than a row of dashes. In practice the API fills all
    /// three on every tracked pitch (6,418 of 6,418 in the sample above), so the
    /// empty case is old games and spring parks without the hardware.
    pub fn describe(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(rpm) = self.spin_rate {
            parts.push(format!("{rpm}rpm"));
        }
        if let Some(ivb) = self.induced_vertical {
            parts.push(format!("IVB {ivb:.0}\""));
        }
        if let Some(hb) = self.horizontal {
            parts.push(format!("HB {:.0}\"{}", hb.abs(), Self::chart_arrow(hb)));
        }
        (!parts.is_empty()).then(|| parts.join("  "))
    }

    /// Which way the break carried the ball *on the strike zone chart*.
    ///
    /// The chart plots `pX` (catcher's perspective) and the API reports horizontal
    /// break from the pitcher's, so the arrow is the negation: positive break — a
    /// right hander's arm side — moves the ball to the chart's left, toward the
    /// right handed batter's box.
    fn chart_arrow(horizontal: f64) -> char {
        if horizontal > 0.0 {
            '\u{2190}'
        } else {
            '\u{2192}'
        }
    }
}

impl Pitch {
    /// Where the pitch crossed the plate, in inches, for the strike zone canvas.
    /// The API reports feet; everything the canvas draws is in inches.
    pub fn plot_position(&self) -> (f64, f64) {
        const SCALE: f64 = 12.0; // feet to inches
        (self.location.0 * SCALE, self.location.1 * SCALE)
    }

    /// Convert a pitch into a TUI Line item, displaying the pitch index, result (ball, strike, ect)
    /// and pitch type (cutter, changeup, ect). For example: "1  Foul | Four-Seam Fastball"
    /// If the pitch is under review, a new line will displayed under the pitch with the player and
    /// team that initiated the review.
    pub fn as_lines(
        &self,
        debug: bool,
        show_movement: bool,
        home_team: &Team,
        away_team: &Team,
        players: &PlayerMap,
    ) -> Vec<Line<'_>> {
        let mut lines = Vec::new();

        // if the pitch is under review, display the review status above the pitch info
        if let Some(review) = &self.review_details
            && let Some(spans) = review.format_in_progress_spans(home_team, away_team, players)
        {
            let mut in_progress_spans = vec![Span::raw(" ")];
            in_progress_spans.extend(spans);
            lines.push(Line::from(in_progress_spans));
        }

        let mut pitch_line_spans = vec![Span::styled(
            format!(" {:<2}", self.index),
            Style::default().fg(self.color),
        )];
        pitch_line_spans.extend(self.format_spans(debug, players));
        // Appended to the pitch's own line rather than given a line of its own: on a
        // wide panel it costs nothing, and the event list only gets 35% of the panel
        // height, so a second line per pitch would halve how many pitches are visible.
        if show_movement && let Some(movement) = self.movement.describe() {
            pitch_line_spans.push(Span::styled(format!("  {movement}"), dim_style()));
        }
        lines.push(Line::from(pitch_line_spans));

        lines
    }

    fn format_spans(&self, debug: bool, players: &PlayerMap) -> Vec<Span<'_>> {
        let base = format!(
            " {:<20}| {}-{} | {:^5.1}| {}",
            self.description, self.count.balls, self.count.strikes, self.speed, self.pitch_type,
        );
        let s = if debug {
            format!(" {} | {:?}", base, self.location)
        } else {
            base
        };
        let mut spans = vec![Span::raw(s)];
        if let Some(review) = &self.review_details
            && let Some(review_spans) = review.format_status_spans(players)
        {
            spans.extend(review_spans);
        }
        spans
    }
}

impl From<&Play> for Pitches {
    fn from(play: &Play) -> Self {
        let pitch_events = play.play_events.iter().map(PitchEvent::from).collect();
        Pitches { pitch_events }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movement(rpm: Option<u32>, ivb: Option<f64>, hb: Option<f64>) -> Movement {
        Movement {
            spin_rate: rpm,
            induced_vertical: ivb,
            horizontal: hb,
        }
    }

    #[test]
    fn a_tracked_pitch_reports_spin_and_both_breaks() {
        let described = movement(Some(2144), Some(12.6), Some(-4.9))
            .describe()
            .unwrap();
        assert_eq!(described, "2144rpm  IVB 13\"  HB 5\"→");
    }

    #[test]
    fn a_right_handers_arm_side_run_points_to_the_right_handed_box() {
        // the API reports horizontal break from the pitcher's perspective, where
        // positive is a righty's arm side; the chart is drawn in pX, the catcher's,
        // where the right handed box is on the left. Measured across 6,418 pitches:
        // RHP four-seamers are 97% positive breakHorizontal and 98% negative pfxX.
        assert_eq!(Movement::chart_arrow(4.9), '←');
        assert_eq!(Movement::chart_arrow(-4.9), '→');
    }

    #[test]
    fn a_dropping_pitch_keeps_its_negative_induced_break() {
        // a curveball's IVB is negative and that sign is the whole point
        let described = movement(Some(2600), Some(-8.4), Some(6.0))
            .describe()
            .unwrap();
        assert!(described.contains("IVB -8\""), "got {described:?}");
    }

    #[test]
    fn a_partly_tracked_pitch_reports_only_what_it_has() {
        assert_eq!(
            movement(Some(2100), None, None).describe().as_deref(),
            Some("2100rpm")
        );
        assert_eq!(
            movement(None, None, Some(3.0)).describe().as_deref(),
            Some("HB 3\"←")
        );
    }

    #[test]
    fn an_untracked_pitch_adds_nothing_to_its_line() {
        // a row of dashes would read as real data; an old game or a park without the
        // hardware should just show the pitch as it always did
        assert_eq!(movement(None, None, None).describe(), None);
    }

    #[test]
    fn a_pitch_with_no_break_block_defaults_to_untracked() {
        assert_eq!(Pitch::default().movement.describe(), None);
    }
}
