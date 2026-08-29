use crate::components::game::live_game::PlayerMap;
use crate::components::game::pitches::Pitch;
use crate::components::standings::Team;
use crate::ui::gameday::plays::{BLUE, SCORING_SYMBOL, build_scoring_span};
use tui::prelude::{Line, Span, Style};

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum PitchEventType {
    Pitch,
    Running,
    /// pickoff attempt, mound visit, pinch hitter, etc
    Other,
}

#[derive(Debug)]
pub struct HitData {
    pub exit_velocity: Option<f64>,
    pub launch_angle: Option<f64>,
    pub distance: Option<f64>,
    #[allow(dead_code)]
    pub hardness: Option<String>,
}

#[derive(Debug)]
pub struct PitchEvent {
    pub event_type: PitchEventType,
    pub description: String,
    /// A lineup change rather than an ordinary in play event. Substitution lines name
    /// players who are not otherwise on screen, so they get jersey numbers.
    pub is_substitution: bool,
    pub pitch: Option<Pitch>,
    pub hit_data: Option<HitData>,
    pub is_scoring: Option<bool>,
    pub away_score: Option<u8>,
    pub home_score: Option<u8>,
}

impl From<&mlbt_api::plays::PlayEvent> for PitchEvent {
    fn from(play: &mlbt_api::plays::PlayEvent) -> Self {
        let pitch = match play.is_pitch {
            true => Some(Pitch::from(play)),
            false => None,
        };
        let hit_data = play.hit_data.as_ref().map(|d| HitData {
            exit_velocity: d.launch_speed,
            launch_angle: d.launch_angle,
            distance: d.total_distance,
            hardness: d.hardness.clone(),
        });
        let event_type = match (play.is_pitch, play.is_base_running_play) {
            (true, _) => PitchEventType::Pitch,
            (false, Some(true)) => PitchEventType::Running,
            (false, _) => PitchEventType::Other,
        };
        Self {
            event_type,
            is_substitution: play.is_substitution.unwrap_or(false),
            description: play.details.description.clone().unwrap_or_default(),
            pitch,
            hit_data,
            is_scoring: play.details.is_scoring_play,
            away_score: play.details.away_score,
            home_score: play.details.home_score,
        }
    }
}

impl PitchEvent {
    /// Convert a pitch event into a TUI Line item.
    /// If it's a pitch, display the pitch information.
    /// Otherwise, display the description.
    pub fn as_lines(
        &self,
        debug: bool,
        show_movement: bool,
        home_team: &Team,
        away_team: &Team,
        players: &PlayerMap,
    ) -> Option<Vec<Line<'_>>> {
        match self.event_type {
            PitchEventType::Pitch if self.pitch.is_some() => self
                .pitch
                .as_ref()
                .map(|pitch| pitch.as_lines(debug, show_movement, home_team, away_team, players)),
            PitchEventType::Pitch => None,
            _ => Some(self.format_non_pitch_event(
                home_team.abbreviation,
                away_team.abbreviation,
                players,
            )),
        }
    }

    fn format_non_pitch_event(
        &self,
        home_team_abbreviation: &'static str,
        away_team_abbreviation: &'static str,
        players: &PlayerMap,
    ) -> Vec<Line<'_>> {
        let mut spans = Vec::new();

        // Add scoring information if this is a scoring event
        let is_scoring = self.is_scoring.unwrap_or(false);
        if is_scoring {
            spans.push(Span::styled(
                format!(" {SCORING_SYMBOL}"),
                Style::default().fg(BLUE),
            ));
        }

        let description = if self.is_substitution {
            number_players(&self.description, players)
        } else {
            self.description.clone()
        };
        spans.push(Span::raw(format!(" {description}")));

        // Add the score at the end of the line if available
        if is_scoring
            && let (Some(away_score), Some(home_score)) = (self.away_score, self.home_score)
        {
            spans.push(build_scoring_span(
                home_score,
                home_team_abbreviation,
                away_score,
                away_team_abbreviation,
            ));
        }

        vec![Line::from(spans)]
    }

    pub fn format_hit_data(&self) -> Option<String> {
        let mut text = String::new();
        if let Some(hit) = &self.hit_data {
            if let Some(exit_velocity) = hit.exit_velocity {
                text.push_str(&format!("exit velo: {exit_velocity}"));
            }

            if let Some(launch_angle) = hit.launch_angle {
                if !text.is_empty() {
                    text.push_str(" | ");
                }
                text.push_str(&format!("LA: {launch_angle}°"));
            }

            if let Some(distance) = hit.distance {
                if !text.is_empty() {
                    text.push_str(" | ");
                }
                text.push_str(&format!("distance: {distance}'"));
            }
            Some(text)
        } else {
            None
        }
    }
}

/// Prefix every player named in a substitution line with his jersey number, so
/// `Pitching Change: Trent Thornton replaces David Peterson.` reads
/// `Pitching Change: #47 Trent Thornton replaces #23 David Peterson.`
///
/// The API only gives an id for the player coming *in* — the one going out is named
/// in prose and nowhere else — so both are found by scanning the text for names in
/// the player map rather than by id.
///
/// Two rules keep that scan honest. Longest name first, so a player whose full name
/// contains another's (a `Jr.`, or a shared first-and-last) claims the match. And
/// matches must fall on word boundaries, so a short name cannot land inside a longer
/// one. Overlapping matches are dropped rather than nested.
fn number_players(description: &str, players: &PlayerMap) -> String {
    let mut candidates: Vec<(&str, &str)> = players
        .values()
        .filter(|p| !p.full_name.is_empty())
        .filter_map(|p| p.number.as_deref().map(|n| (p.full_name.as_str(), n)))
        .collect();
    candidates.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));

    let mut found: Vec<(usize, usize, &str)> = Vec::new();
    for (name, number) in candidates {
        let mut from = 0;
        while let Some(offset) = description[from..].find(name) {
            let start = from + offset;
            let end = start + name.len();
            from = end;
            if !on_word_boundary(description, start, end) {
                continue;
            }
            if found.iter().any(|(s, e, _)| start < *e && *s < end) {
                continue;
            }
            found.push((start, end, number));
        }
    }
    if found.is_empty() {
        return description.to_string();
    }
    found.sort_by_key(|(start, _, _)| *start);

    let mut out = String::with_capacity(description.len() + found.len() * 5);
    let mut cursor = 0;
    for (start, end, number) in found {
        out.push_str(&description[cursor..start]);
        out.push('#');
        out.push_str(number);
        out.push(' ');
        out.push_str(&description[start..end]);
        cursor = end;
    }
    out.push_str(&description[cursor..]);
    out
}

fn on_word_boundary(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::game::player::Player;
    use std::collections::HashMap;

    fn map(entries: &[(u64, &str, Option<&str>)]) -> PlayerMap {
        let mut m: PlayerMap = HashMap::new();
        for (id, full_name, number) in entries {
            m.insert(
                *id,
                Player {
                    id: *id,
                    full_name: (*full_name).to_string(),
                    number: number.map(|n| n.to_string()),
                    ..Default::default()
                },
            );
        }
        m
    }

    #[test]
    fn a_pitching_change_numbers_both_pitchers() {
        let players = map(&[
            (1, "Trent Thornton", Some("47")),
            (2, "David Peterson", Some("23")),
        ]);
        assert_eq!(
            number_players(
                "Pitching Change: Trent Thornton replaces David Peterson.",
                &players
            ),
            "Pitching Change: #47 Trent Thornton replaces #23 David Peterson."
        );
    }

    #[test]
    fn a_defensive_switch_numbers_its_one_player() {
        let players = map(&[(1, "JJ Bleday", Some("33"))]);
        assert_eq!(
            number_players(
                "Defensive switch from left field to right field for JJ Bleday.",
                &players
            ),
            "Defensive switch from left field to right field for #33 JJ Bleday."
        );
    }

    #[test]
    fn a_player_with_no_number_is_left_alone() {
        // rather than printing a bare `#`
        let players = map(&[
            (1, "Trent Thornton", None),
            (2, "David Peterson", Some("23")),
        ]);
        assert_eq!(
            number_players(
                "Pitching Change: Trent Thornton replaces David Peterson.",
                &players
            ),
            "Pitching Change: Trent Thornton replaces #23 David Peterson."
        );
    }

    #[test]
    fn the_longer_name_wins_an_overlap() {
        // a suffix must not be left stranded by matching the shorter name first
        let players = map(&[
            (1, "Luis Garcia", Some("2")),
            (2, "Luis Garcia Jr.", Some("77")),
        ]);
        assert_eq!(
            number_players(
                "Pitching Change: Luis Garcia Jr. replaces Ryan Yarbrough.",
                &players
            ),
            "Pitching Change: #77 Luis Garcia Jr. replaces Ryan Yarbrough."
        );
    }

    #[test]
    fn a_name_inside_a_longer_word_is_not_matched() {
        let players = map(&[(1, "Ty France", Some("13"))]);
        let line = "Offensive Substitution: Pinch-hitter Tyler Franceschi replaces someone.";
        assert_eq!(number_players(line, &players), line);
    }

    #[test]
    fn a_line_naming_nobody_known_is_unchanged() {
        let players = map(&[(1, "Trent Thornton", Some("47"))]);
        let line = "Mound visit.";
        assert_eq!(number_players(line, &players), line);
    }

    #[test]
    fn accented_names_survive_the_scan() {
        // byte offsets, not char offsets, are what the scan slices on
        let players = map(&[
            (1, "Jose Ramirez", Some("11")),
            (2, "Andres Munoz", Some("75")),
        ]);
        assert_eq!(
            number_players(
                "Offensive Substitution: Jose Ramirez replaces Andres Munoz.",
                &players
            ),
            "Offensive Substitution: #11 Jose Ramirez replaces #75 Andres Munoz."
        );
    }

    #[test]
    fn only_substitutions_are_annotated() {
        let players = map(&[(1, "Trent Thornton", Some("47"))]);
        let event = PitchEvent {
            event_type: PitchEventType::Other,
            is_substitution: false,
            description: "Wild pitch by Trent Thornton.".to_string(),
            pitch: None,
            hit_data: None,
            is_scoring: None,
            away_score: None,
            home_score: None,
        };
        let lines = event.format_non_pitch_event("NYY", "BOS", &players);
        assert_eq!(lines[0].to_string(), " Wild pitch by Trent Thornton.");
    }

    #[test]
    fn a_substitution_event_is_annotated_end_to_end() {
        let players = map(&[(1, "Trent Thornton", Some("47"))]);
        let event = PitchEvent {
            event_type: PitchEventType::Other,
            is_substitution: true,
            description: "Pitching Change: Trent Thornton.".to_string(),
            pitch: None,
            hit_data: None,
            is_scoring: None,
            away_score: None,
            home_score: None,
        };
        let lines = event.format_non_pitch_event("NYY", "BOS", &players);
        assert_eq!(
            lines[0].to_string(),
            " Pitching Change: #47 Trent Thornton."
        );
    }
}
