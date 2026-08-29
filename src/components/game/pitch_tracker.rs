//! Per-pitcher totals accumulated across the whole game.
//!
//! The at bat panel already shows every pitch of the *current* at bat. This is the
//! other half of the question you actually ask while watching: what has this guy
//! been throwing all night, and how hard. It aggregates every pitch the pitcher of
//! the viewed at bat has thrown in this game, grouped by pitch type.
//!
//! Totals run *through* the viewed at bat, not through the whole game, so scrolling
//! back to the third inning shows what the pitcher had thrown by then rather than
//! what he finished with. Live — where the viewed at bat is the current one — the
//! two are the same thing.

use crate::components::game::live_game::GameState;

/// A pitch type and everything worth knowing about how often it has been thrown.
#[derive(Debug, Clone, PartialEq)]
pub struct PitchTypeTotals {
    /// Short API code, e.g. `FF`, `SL`. `??` when the API did not classify the pitch.
    pub code: String,
    pub count: usize,
    /// Share of this pitcher's pitches, 0-100.
    pub share: f64,
    pub avg_speed: f64,
    pub max_speed: f64,
}

/// Everything the tracker column shows for one pitcher.
#[derive(Debug, Clone, PartialEq)]
pub struct PitcherTotals {
    pub pitcher_id: u64,
    pub pitches: usize,
    pub strikes: usize,
    /// Pitch types sorted by count, most thrown first.
    pub types: Vec<PitchTypeTotals>,
}

/// Code shown when the API classified a pitch as `UNKNOWN` or sent no type at all.
const UNCLASSIFIED: &str = "??";

impl PitcherTotals {
    /// Strike percentage, 0-100. Zero when nothing has been thrown, which reads
    /// correctly as "no data" rather than dividing by zero.
    pub fn strike_share(&self) -> f64 {
        if self.pitches == 0 {
            0.0
        } else {
            self.strikes as f64 * 100.0 / self.pitches as f64
        }
    }
}

impl GameState {
    /// Totals for the pitcher of the at bat at `through_index`, counting every pitch
    /// he has thrown in this game up to and including that at bat.
    ///
    /// Returns `None` when the at bat is unknown or has no pitcher yet (a game that
    /// has not started), so the caller can leave the column empty rather than render
    /// a row of zeroes that looks like real data.
    pub fn pitcher_totals_through(&self, through_index: u8) -> Option<PitcherTotals> {
        let pitcher_id = self.get_at_bat_by_index(through_index)?.matchup.pitcher_id;
        if pitcher_id == 0 {
            return None;
        }

        let mut pitches = 0usize;
        let mut strikes = 0usize;
        // insertion ordered so equal counts stay in the order first thrown, which
        // keeps the column from reshuffling on every refresh
        let mut by_type: Vec<(String, Vec<f64>)> = Vec::new();

        let relevant = self
            .at_bats
            .iter()
            .filter(|(index, at_bat)| {
                **index <= through_index && at_bat.matchup.pitcher_id == pitcher_id
            })
            .flat_map(|(_, at_bat)| at_bat.pitches.pitches.pitch_events.iter())
            .filter_map(|event| event.pitch.as_ref());

        for pitch in relevant {
            pitches += 1;
            if pitch.strike {
                strikes += 1;
            }
            let code = normalize_code(&pitch.pitch_type_code);
            match by_type.iter_mut().find(|(existing, _)| *existing == code) {
                Some((_, speeds)) => speeds.push(pitch.speed),
                None => by_type.push((code, vec![pitch.speed])),
            }
        }

        if pitches == 0 {
            return Some(PitcherTotals {
                pitcher_id,
                pitches: 0,
                strikes: 0,
                types: Vec::new(),
            });
        }

        let mut types: Vec<PitchTypeTotals> = by_type
            .into_iter()
            .map(|(code, speeds)| {
                // a pitch with no tracked velocity reports 0.0; averaging those in
                // would drag a real average down, so they are excluded from the
                // speed columns while still counting toward the pitch count
                let tracked: Vec<f64> = speeds.iter().copied().filter(|s| *s > 0.0).collect();
                let avg_speed = if tracked.is_empty() {
                    0.0
                } else {
                    tracked.iter().sum::<f64>() / tracked.len() as f64
                };
                let max_speed = tracked.iter().copied().fold(0.0, f64::max);
                PitchTypeTotals {
                    code,
                    count: speeds.len(),
                    share: speeds.len() as f64 * 100.0 / pitches as f64,
                    avg_speed,
                    max_speed,
                }
            })
            .collect();
        // most thrown first; the sort is stable so ties keep first-thrown order
        types.sort_by_key(|t| std::cmp::Reverse(t.count));

        Some(PitcherTotals {
            pitcher_id,
            pitches,
            strikes,
            types,
        })
    }
}

/// Collapse the API's missing-type sentinel and any empty string to one code, so the
/// column never prints `UNKNOWN` (seven characters into a column that holds two).
fn normalize_code(code: &str) -> String {
    match code.trim() {
        "" | "UNKNOWN" => UNCLASSIFIED.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::game::at_bat::AtBat;
    use crate::components::game::pitch_event::{PitchEvent, PitchEventType};
    use crate::components::game::pitches::Pitch;

    fn pitch(code: &str, speed: f64, strike: bool) -> PitchEvent {
        PitchEvent {
            event_type: PitchEventType::Pitch,
            description: String::new(),
            pitch: Some(Pitch {
                strike,
                speed,
                pitch_type_code: code.to_string(),
                ..Default::default()
            }),
            hit_data: None,
            is_scoring: None,
            away_score: None,
            home_score: None,
        }
    }

    /// A non-pitch event (a pickoff, a mound visit) that must not count as a pitch.
    fn running_event() -> PitchEvent {
        PitchEvent {
            event_type: PitchEventType::Running,
            description: "Pickoff attempt".to_string(),
            pitch: None,
            hit_data: None,
            is_scoring: None,
            away_score: None,
            home_score: None,
        }
    }

    fn game_with(at_bats: Vec<(u8, u64, Vec<PitchEvent>)>) -> GameState {
        let mut game = GameState::default();
        for (index, pitcher_id, events) in at_bats {
            let mut at_bat = AtBat {
                index,
                ..Default::default()
            };
            at_bat.matchup.pitcher_id = pitcher_id;
            at_bat.pitches.pitches.pitch_events = events;
            game.at_bats.insert(index, at_bat);
        }
        game
    }

    #[test]
    fn totals_cover_every_at_bat_the_pitcher_worked() {
        let game = game_with(vec![
            (
                0,
                100,
                vec![pitch("FF", 95.0, true), pitch("SL", 86.0, false)],
            ),
            (1, 100, vec![pitch("FF", 97.0, true)]),
        ]);

        let totals = game.pitcher_totals_through(1).unwrap();
        assert_eq!(totals.pitches, 3);
        assert_eq!(totals.strikes, 2);
        assert_eq!(totals.types[0].code, "FF");
        assert_eq!(totals.types[0].count, 2);
        assert_eq!(totals.types[0].avg_speed, 96.0);
        assert_eq!(totals.types[0].max_speed, 97.0);
    }

    #[test]
    fn a_relief_appearance_does_not_inherit_the_starters_pitches() {
        let game = game_with(vec![
            (
                0,
                100,
                vec![pitch("FF", 95.0, true), pitch("FF", 96.0, true)],
            ),
            (1, 200, vec![pitch("SL", 84.0, false)]),
        ]);

        let reliever = game.pitcher_totals_through(1).unwrap();
        assert_eq!(reliever.pitcher_id, 200);
        assert_eq!(reliever.pitches, 1, "only his own pitch");
    }

    #[test]
    fn totals_stop_at_the_viewed_at_bat() {
        // scrolling back to an earlier at bat should show what the pitcher had thrown
        // by then, not what he finished the night with
        let game = game_with(vec![
            (0, 100, vec![pitch("FF", 95.0, true)]),
            (1, 100, vec![pitch("FF", 96.0, true)]),
            (2, 100, vec![pitch("FF", 97.0, true)]),
        ]);

        assert_eq!(game.pitcher_totals_through(0).unwrap().pitches, 1);
        assert_eq!(game.pitcher_totals_through(2).unwrap().pitches, 3);
    }

    #[test]
    fn non_pitch_events_are_not_counted() {
        let game = game_with(vec![(
            0,
            100,
            vec![pitch("FF", 95.0, true), running_event()],
        )]);

        assert_eq!(game.pitcher_totals_through(0).unwrap().pitches, 1);
    }

    #[test]
    fn shares_are_percentages_of_the_pitchers_own_total() {
        let game = game_with(vec![(
            0,
            100,
            vec![
                pitch("FF", 95.0, true),
                pitch("FF", 95.0, true),
                pitch("FF", 95.0, true),
                pitch("CH", 85.0, false),
            ],
        )]);

        let totals = game.pitcher_totals_through(0).unwrap();
        assert_eq!(totals.types[0].share, 75.0);
        assert_eq!(totals.types[1].share, 25.0);
        assert_eq!(totals.strike_share(), 75.0);
    }

    #[test]
    fn untracked_velocity_does_not_drag_the_average_down() {
        // a pitch the API sent no speed for reports 0.0; counting it would report a
        // fastball averaging 48 mph
        let game = game_with(vec![(
            0,
            100,
            vec![pitch("FF", 96.0, true), pitch("FF", 0.0, true)],
        )]);

        let totals = game.pitcher_totals_through(0).unwrap();
        assert_eq!(totals.types[0].count, 2, "still counts as a pitch");
        assert_eq!(totals.types[0].avg_speed, 96.0);
    }

    #[test]
    fn unclassified_pitches_get_a_short_code() {
        let game = game_with(vec![(
            0,
            100,
            vec![pitch("UNKNOWN", 90.0, true), pitch("", 90.0, true)],
        )]);

        let totals = game.pitcher_totals_through(0).unwrap();
        assert_eq!(totals.types.len(), 1, "both collapse to one bucket");
        assert_eq!(totals.types[0].code, UNCLASSIFIED);
    }

    #[test]
    fn types_are_ordered_by_how_often_they_were_thrown() {
        let game = game_with(vec![(
            0,
            100,
            vec![
                pitch("CH", 85.0, false),
                pitch("FF", 95.0, true),
                pitch("FF", 95.0, true),
            ],
        )]);

        let totals = game.pitcher_totals_through(0).unwrap();
        let codes: Vec<&str> = totals.types.iter().map(|t| t.code.as_str()).collect();
        assert_eq!(codes, vec!["FF", "CH"]);
    }

    #[test]
    fn an_unknown_at_bat_has_no_totals() {
        let game = game_with(vec![]);
        assert!(game.pitcher_totals_through(0).is_none());
    }

    #[test]
    fn a_pitcher_who_has_not_thrown_yet_reports_zero_rather_than_nothing() {
        // the column should say "0 P" for a fresh reliever, not vanish
        let game = game_with(vec![(0, 100, vec![])]);
        let totals = game.pitcher_totals_through(0).unwrap();
        assert_eq!(totals.pitches, 0);
        assert!(totals.types.is_empty());
        assert_eq!(totals.strike_share(), 0.0);
    }
}
