//! What the viewer has actually seen.
//!
//! Gameday is walked one pitch at a time so a delayed broadcast is never spoiled. That
//! only works if *every* panel renders the game as of that pitch rather than as of now:
//! a truncated pitch list beside a live linescore still gives the inning away, and the
//! scoreboard reading the play's own end-of-play score announces the home run before the
//! pitch that produced it has been drawn.
//!
//! This module is the one place that answers "what did the game look like here", so the
//! widgets do not each invent their own answer.

use crate::components::game::at_bat::AtBat;
use crate::components::game::live_game::GameState;
use crate::components::game::matchup::Runners;
use crate::components::game::pitch_event::PitchEvent;
use crate::components::linescore::{LineScore, LineScoreLine};
use crate::state::app_state::HomeOrAway;

/// Where the viewer has walked to: an at bat, and how far into it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub at_bat: u8,
    /// Inclusive index into that at bat's `pitch_events`. An index rather than the API's
    /// `pitch_number` because substitutions, pickoffs and mound visits share the vec and
    /// carry no number.
    pub pitch: usize,
}

/// The at bats that are visible at all. `at_bats` is in game order, so anything past the
/// cursor's at bat has not been reached.
pub fn is_revealed(at_bat_index: u8, cursor: Option<Cursor>) -> bool {
    match cursor {
        Some(c) => at_bat_index <= c.at_bat,
        None => true,
    }
}

/// The events of `at_bat` the viewer has reached. Earlier at bats are whole; the cursor's
/// own at bat is cut at the pitch it is sitting on.
pub fn revealed_events(at_bat: &AtBat, cursor: Option<Cursor>) -> &[PitchEvent] {
    let events = &at_bat.pitches.pitches.pitch_events;
    match cursor {
        Some(c) if c.at_bat == at_bat.index => &events[..(c.pitch + 1).min(events.len())],
        _ => events,
    }
}

/// Whether the at bat has been walked to its final event, which is the moment its outcome
/// — the result line, the hit data, the run it scored — stops being a spoiler.
pub fn at_bat_complete(at_bat: &AtBat, cursor: Option<Cursor>) -> bool {
    match cursor {
        Some(c) if c.at_bat == at_bat.index => {
            let len = at_bat.pitches.pitches.pitch_events.len();
            len == 0 || c.pitch + 1 >= len
        }
        _ => true,
    }
}

/// The game situation as of the revealed pitch: what a scoreboard would have shown while
/// the pitch was in the air, rather than after the play was over.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Situation {
    pub away_score: u8,
    pub home_score: u8,
    pub inning: u8,
    pub is_top: bool,
    pub balls: u8,
    pub strikes: u8,
    pub outs: u8,
    pub runners: Runners,
}

/// Rebuild the scoreboard for `at_bat` as of the cursor.
///
/// A completed at bat is just its own end-of-play state. A partial one has to be assembled:
/// the score is carried in from the last play that finished (individual pitches only carry
/// a score on the action events that change it), the count comes off the last pitch thrown,
/// and the runners come from the *previous* play, because the API's `post_on_first/second/third`
/// describe where everyone ended up after this play — the very thing being hidden.
pub fn situation_at(game: &GameState, at_bat: &AtBat, cursor: Option<Cursor>) -> Situation {
    let matchup = &at_bat.matchup;
    if at_bat_complete(at_bat, cursor) {
        return Situation {
            away_score: matchup.away_score,
            home_score: matchup.home_score,
            inning: matchup.inning,
            is_top: matchup.is_top,
            balls: matchup.count.balls,
            strikes: matchup.count.strikes,
            outs: matchup.count.outs,
            runners: matchup.runners,
        };
    }

    let events = revealed_events(at_bat, cursor);
    let previous = previous_at_bat(game, at_bat.index);

    // Score entering the at bat, then any run an already-revealed action event booked.
    let (mut away_score, mut home_score) = previous
        .map(|p| (p.play_result.away_score, p.play_result.home_score))
        .unwrap_or((0, 0));
    for event in events {
        if let (Some(a), Some(h)) = (event.away_score, event.home_score) {
            away_score = a;
            home_score = h;
        }
    }

    // Count off the last pitch actually thrown. Before the first pitch the at bat is 0-0,
    // and the outs it started with come from the first pitch of the at bat — which is the
    // state entering it, not an outcome.
    let last_pitch = events.iter().rev().find_map(|e| e.pitch.as_ref());
    let first_pitch = at_bat
        .pitches
        .pitches
        .pitch_events
        .iter()
        .find_map(|e| e.pitch.as_ref());
    let (balls, strikes) = last_pitch
        .map(|p| (p.count.balls, p.count.strikes))
        .unwrap_or((0, 0));
    let outs = last_pitch
        .or(first_pitch)
        .map(|p| p.count.outs)
        .unwrap_or(matchup.count.outs);

    // Runners the previous play left on, unless it was the other half of the inning, when
    // the bases were cleared between them.
    let runners = previous
        .filter(|p| p.inning == at_bat.inning && p.is_top_inning == at_bat.is_top_inning)
        .map(|p| p.matchup.runners)
        .unwrap_or_default();

    Situation {
        away_score,
        home_score,
        inning: matchup.inning,
        is_top: matchup.is_top,
        balls,
        strikes,
        outs,
        runners,
    }
}

fn previous_at_bat(game: &GameState, index: u8) -> Option<&AtBat> {
    game.at_bats
        .values()
        .take_while(|at_bat| at_bat.index < index)
        .last()
}

/// Rebuild the linescore from the plays the viewer has reached.
///
/// Returns `None` when live, where the API's own linescore is already correct and is the
/// more authoritative source. Runs come from the score carried on each play, so they need
/// no classification; hits and errors are counted off the play's `eventType`, which does
/// not capture every scorer's decision but is right for the ordinary ones.
pub fn linescore_through(game: &GameState, cursor: Option<Cursor>) -> Option<LineScore> {
    let cursor = cursor?;

    let mut away = new_line(game.away_team.abbreviation, HomeOrAway::Away);
    let mut home = new_line(game.home_team.abbreviation, HomeOrAway::Home);
    let (mut previous_away, mut previous_home) = (0u8, 0u8);
    let mut latest_inning = 1u8;

    for at_bat in game.at_bats.values() {
        if !is_revealed(at_bat.index, cursor.into()) {
            break;
        }
        let complete = at_bat_complete(at_bat, cursor.into());
        latest_inning = latest_inning.max(at_bat.inning);

        let inning = at_bat.inning as usize;
        while away.inning_score.len() < inning {
            away.inning_score.push(0);
            home.inning_score.push(0);
        }

        let (away_score, home_score) = if complete {
            (at_bat.play_result.away_score, at_bat.play_result.home_score)
        } else {
            revealed_events(at_bat, cursor.into())
                .iter()
                .rev()
                .find_map(|e| e.away_score.zip(e.home_score))
                .unwrap_or((previous_away, previous_home))
        };

        away.inning_score[inning - 1] += away_score.saturating_sub(previous_away);
        home.inning_score[inning - 1] += home_score.saturating_sub(previous_home);
        away.runs = away_score;
        home.runs = home_score;
        previous_away = away_score;
        previous_home = home_score;

        if complete {
            // The batting team gets the hit; the team in the field wears the error.
            let (batting, fielding) = match at_bat.is_top_inning {
                true => (&mut away, &mut home),
                false => (&mut home, &mut away),
            };
            if at_bat.play_result.is_hit() {
                batting.hits += 1;
            }
            if at_bat.play_result.is_error() {
                fielding.errors += 1;
            }
        }
    }

    Some(LineScore {
        header: LineScoreLine::create_header_vec(latest_inning),
        away,
        home,
        mini: true,
    })
}

fn new_line(abbreviation: &str, team: HomeOrAway) -> LineScoreLine {
    LineScoreLine {
        team,
        abbreviation: abbreviation.to_string(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::game::pitches::{Count, Pitch};

    fn pitch_event(balls: u8, strikes: u8, outs: u8) -> PitchEvent {
        PitchEvent {
            pitch: Some(Pitch {
                count: Count {
                    balls,
                    strikes,
                    outs,
                },
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// An at bat with `pitches` pitches, ending at the given score.
    fn at_bat(index: u8, inning: u8, is_top: bool, pitches: usize, away: u8, home: u8) -> AtBat {
        let mut at_bat = AtBat {
            index,
            inning,
            is_top_inning: is_top,
            ..Default::default()
        };
        at_bat.pitches.pitches.pitch_events =
            (0..pitches).map(|i| pitch_event(0, i as u8, 1)).collect();
        at_bat.play_result.away_score = away;
        at_bat.play_result.home_score = home;
        at_bat.matchup.away_score = away;
        at_bat.matchup.home_score = home;
        at_bat.matchup.inning = inning;
        at_bat.matchup.is_top = is_top;
        at_bat
    }

    fn game_with(at_bats: Vec<AtBat>) -> GameState {
        let mut game = GameState::default();
        for ab in at_bats {
            game.at_bats.insert(ab.index, ab);
        }
        game
    }

    #[test]
    fn a_partial_at_bat_shows_only_the_pitches_reached() {
        let ab = at_bat(0, 1, true, 4, 0, 0);
        let cursor = Some(Cursor {
            at_bat: 0,
            pitch: 1,
        });
        assert_eq!(revealed_events(&ab, cursor).len(), 2);
        assert!(!at_bat_complete(&ab, cursor));
    }

    #[test]
    fn walking_to_the_last_pitch_completes_the_at_bat() {
        // the outcome stops being a spoiler exactly here, and not one pitch earlier
        let ab = at_bat(0, 1, true, 4, 0, 0);
        let cursor = Some(Cursor {
            at_bat: 0,
            pitch: 3,
        });
        assert_eq!(revealed_events(&ab, cursor).len(), 4);
        assert!(at_bat_complete(&ab, cursor));
    }

    #[test]
    fn an_earlier_at_bat_is_whole() {
        let ab = at_bat(0, 1, true, 4, 0, 0);
        let cursor = Some(Cursor {
            at_bat: 5,
            pitch: 0,
        });
        assert_eq!(revealed_events(&ab, cursor).len(), 4);
        assert!(at_bat_complete(&ab, cursor));
    }

    #[test]
    fn a_partial_at_bat_carries_the_score_in_rather_than_showing_its_own() {
        // at bat 1 ends 0-3; standing on its second pitch must still read 0-0
        let game = game_with(vec![
            at_bat(0, 1, false, 3, 0, 0),
            at_bat(1, 1, false, 5, 0, 3),
        ]);
        let cursor = Some(Cursor {
            at_bat: 1,
            pitch: 1,
        });
        let situation = situation_at(&game, game.get_at_bat_by_index(1).unwrap(), cursor);

        assert_eq!((situation.away_score, situation.home_score), (0, 0));
        assert_eq!(
            situation.strikes, 1,
            "count comes off the last pitch thrown"
        );
    }

    #[test]
    fn finishing_the_at_bat_releases_its_score() {
        let game = game_with(vec![
            at_bat(0, 1, false, 3, 0, 0),
            at_bat(1, 1, false, 5, 0, 3),
        ]);
        let cursor = Some(Cursor {
            at_bat: 1,
            pitch: 4,
        });
        let situation = situation_at(&game, game.get_at_bat_by_index(1).unwrap(), cursor);

        assert_eq!((situation.away_score, situation.home_score), (0, 3));
    }

    #[test]
    fn runners_come_from_the_previous_play_not_this_one() {
        let mut game = game_with(vec![
            at_bat(0, 1, true, 3, 0, 0),
            at_bat(1, 1, true, 4, 0, 0),
        ]);
        // the previous play left a man on first; this play's own post-runners are empty
        game.at_bats.get_mut(&0).unwrap().matchup.runners = Runners {
            first: true,
            second: false,
            third: false,
        };
        let cursor = Some(Cursor {
            at_bat: 1,
            pitch: 0,
        });
        let situation = situation_at(&game, game.get_at_bat_by_index(1).unwrap(), cursor);

        assert!(situation.runners.first);
    }

    #[test]
    fn the_bases_are_empty_across_a_half_inning_boundary() {
        // whoever was stranded in the top of the first is not still standing there in the
        // bottom of it
        let mut game = game_with(vec![
            at_bat(0, 1, true, 3, 0, 0),
            at_bat(1, 1, false, 4, 0, 0),
        ]);
        game.at_bats.get_mut(&0).unwrap().matchup.runners = Runners {
            first: true,
            second: true,
            third: false,
        };
        let cursor = Some(Cursor {
            at_bat: 1,
            pitch: 0,
        });
        let situation = situation_at(&game, game.get_at_bat_by_index(1).unwrap(), cursor);

        assert!(!situation.runners.first && !situation.runners.second);
    }

    #[test]
    fn the_linescore_stops_at_the_cursor() {
        // three innings played, cursor in the second: the third must not be on the board
        let game = game_with(vec![
            at_bat(0, 1, true, 3, 1, 0),
            at_bat(1, 2, true, 3, 3, 0),
            at_bat(2, 3, true, 3, 7, 0),
        ]);
        let cursor = Some(Cursor {
            at_bat: 1,
            pitch: 2,
        });
        let linescore = linescore_through(&game, cursor).unwrap();

        assert_eq!(linescore.away.inning_score, vec![1, 2]);
        assert_eq!(linescore.away.runs, 3);
    }

    #[test]
    fn a_run_scored_mid_at_bat_is_not_on_the_board_yet() {
        let game = game_with(vec![
            at_bat(0, 1, true, 3, 0, 0),
            at_bat(1, 1, true, 5, 4, 0),
        ]);
        let cursor = Some(Cursor {
            at_bat: 1,
            pitch: 1,
        });
        let linescore = linescore_through(&game, cursor).unwrap();

        assert_eq!(linescore.away.runs, 0);
        assert_eq!(linescore.away.inning_score, vec![0]);
    }

    #[test]
    fn hits_and_errors_are_counted_from_the_plays_reached() {
        let mut game = game_with(vec![
            at_bat(0, 1, true, 3, 0, 0),
            at_bat(1, 1, true, 3, 0, 0),
            at_bat(2, 1, true, 3, 0, 0),
        ]);
        game.at_bats.get_mut(&0).unwrap().play_result.event_type = Some("single".into());
        game.at_bats.get_mut(&1).unwrap().play_result.event_type = Some("field_error".into());
        // not reached, must not be counted
        game.at_bats.get_mut(&2).unwrap().play_result.event_type = Some("home_run".into());

        let linescore = linescore_through(
            &game,
            Some(Cursor {
                at_bat: 1,
                pitch: 2,
            }),
        )
        .unwrap();

        assert_eq!(linescore.away.hits, 1, "the away team is batting");
        assert_eq!(linescore.home.errors, 1, "the fielding team wears it");
    }

    #[test]
    fn live_defers_to_the_api_linescore() {
        let game = game_with(vec![at_bat(0, 1, true, 3, 1, 0)]);
        assert!(linescore_through(&game, None).is_none());
    }

    #[test]
    fn an_at_bat_with_no_events_is_complete() {
        // an at bat announced before its first pitch: nothing to walk through, and treating
        // it as incomplete would strand the cursor on it forever
        let ab = at_bat(0, 1, true, 0, 0, 0);
        let cursor = Some(Cursor {
            at_bat: 0,
            pitch: 0,
        });
        assert!(at_bat_complete(&ab, cursor));
        assert!(revealed_events(&ab, cursor).is_empty());
    }
}

/// Checks against a real game rather than hand-built at bats. `live.json` is a complete
/// Reds-Cubs game, so it exercises the shapes synthetic tests miss: at bats with no pitches,
/// action events between pitches, half innings that strand runners, and a play whose score
/// only appears on its result.
#[cfg(test)]
mod real_game_tests {
    use super::*;
    use mlbt_api::live::LiveResponse;
    use mlbt_api::win_probability::WinProbabilityResponse;

    fn real_game() -> GameState {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/api/tests/responses/live.json"
        ))
        .expect("fixture");
        let response: LiveResponse = serde_json::from_str(&raw).expect("parse");
        let mut game = GameState::default();
        game.update(&response, &WinProbabilityResponse::default());
        game
    }

    /// Every position the cursor can take, in order.
    fn every_position(game: &GameState) -> Vec<Cursor> {
        game.at_bats
            .values()
            .flat_map(|at_bat| {
                let count = at_bat.pitches.pitches.pitch_events.len().max(1);
                (0..count).map(move |pitch| Cursor {
                    at_bat: at_bat.index,
                    pitch,
                })
            })
            .collect()
    }

    #[test]
    fn the_fixture_is_a_real_game() {
        let game = real_game();
        assert_eq!(game.at_bats.len(), 71);
        assert!(
            every_position(&game).len() > 200,
            "a game's worth of pitches"
        );
    }

    #[test]
    fn no_position_ever_shows_a_score_the_game_had_not_reached() {
        // the invariant the whole feature rests on: walking the game forward, the scoreboard
        // must never be ahead of the last completed play
        let game = real_game();
        let mut highest_finished = (0u8, 0u8);

        for cursor in every_position(&game) {
            let at_bat = game.get_at_bat_by_index(cursor.at_bat).unwrap();
            let situation = situation_at(&game, at_bat, Some(cursor));

            if at_bat_complete(at_bat, Some(cursor)) {
                highest_finished = (at_bat.play_result.away_score, at_bat.play_result.home_score);
            } else {
                assert!(
                    situation.away_score <= highest_finished.0
                        && situation.home_score <= highest_finished.1,
                    "at bat {} pitch {} showed {:?}, ahead of the finished {:?}",
                    cursor.at_bat,
                    cursor.pitch,
                    (situation.away_score, situation.home_score),
                    highest_finished
                );
            }
        }
    }

    #[test]
    fn the_linescore_never_runs_ahead_of_the_scoreboard() {
        let game = real_game();
        for cursor in every_position(&game) {
            let linescore = linescore_through(&game, Some(cursor)).unwrap();
            let at_bat = game.get_at_bat_by_index(cursor.at_bat).unwrap();

            assert_eq!(
                linescore.away.runs,
                linescore.away.inning_score.iter().sum::<u8>(),
                "away total must equal its innings"
            );
            assert_eq!(
                linescore.home.runs,
                linescore.home.inning_score.iter().sum::<u8>(),
                "home total must equal its innings"
            );
            assert!(
                linescore.away.inning_score.len() <= at_bat.inning as usize,
                "innings past the cursor must stay blank"
            );
        }
    }

    #[test]
    fn the_final_position_matches_the_api_linescore() {
        // walking to the last pitch of the last play should land exactly on the real game
        let game = real_game();
        let last = *every_position(&game).last().unwrap();
        let walked = linescore_through(&game, Some(last)).unwrap();

        assert_eq!(walked.away.runs, game.linescore.away.runs);
        assert_eq!(walked.home.runs, game.linescore.home.runs);
        assert_eq!(walked.away.hits, game.linescore.away.hits);
        assert_eq!(walked.home.hits, game.linescore.home.hits);
    }

    #[test]
    fn a_scoring_play_stays_hidden_until_its_last_pitch() {
        let game = real_game();
        let scoring = game
            .at_bats
            .values()
            .find(|at_bat| {
                at_bat.play_result.is_scoring_play && at_bat.pitches.pitches.pitch_events.len() > 1
            })
            .expect("a multi pitch scoring play");
        let before = Cursor {
            at_bat: scoring.index,
            pitch: scoring.pitches.pitches.pitch_events.len() - 2,
        };
        let after = Cursor {
            at_bat: scoring.index,
            pitch: scoring.pitches.pitches.pitch_events.len() - 1,
        };

        let hidden = situation_at(&game, scoring, Some(before));
        let shown = situation_at(&game, scoring, Some(after));

        assert_ne!(
            (hidden.away_score, hidden.home_score),
            (shown.away_score, shown.home_score),
            "the run must arrive on the pitch that drove it in, not before"
        );
        assert_eq!(
            (shown.away_score, shown.home_score),
            (
                scoring.play_result.away_score,
                scoring.play_result.home_score
            )
        );
    }
}
