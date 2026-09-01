use crate::components::game::live_game::GameState;
use crate::components::game::matchup::format_scoreboard_lines;
use crate::components::game::reveal::{Cursor, situation_at};
use crate::ui::styling::border_style;
use tui::prelude::*;
use tui::widgets::{Block, Borders, Padding, Paragraph};

pub struct MatchupWidget<'a> {
    pub game: &'a GameState,
    pub selected_at_bat: Option<u8>,
    pub cursor: Option<Cursor>,
}

impl Widget for MatchupWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (at_bat, is_current) = self
            .game
            .get_at_bat_by_index_or_current(self.selected_at_bat);
        let situation = situation_at(self.game, at_bat, self.cursor);
        // A player's game line comes from the boxscore, which is cumulative and live: a
        // pitcher's pitch count and a batter's 2-for-3 are both true *now*, not as of the
        // pitch on screen. They are only safe once the viewer has caught up.
        let live_details = is_current && self.cursor.is_none();

        let [matchup, on_deck] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        let [away, scoreboard, home] = Layout::horizontal([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .horizontal_margin(2)
        .areas(matchup);

        Widget::render(
            Paragraph::new(at_bat.matchup.format_team_lines(
                self.game.away_team.team_name,
                self.game.away_abs_challenges,
                false,
                live_details,
                &self.game.players,
            ))
            .alignment(Alignment::Left)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(border_style())
                    .padding(Padding::new(0, 0, 1, 0)),
            ),
            away,
            buf,
        );
        Widget::render(
            Paragraph::new(format_scoreboard_lines(
                situation.away_score,
                situation.home_score,
                situation.inning,
                situation.is_top,
                situation.outs,
                situation.runners,
            ))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(border_style())
                    .padding(Padding::new(0, 0, 1, 0)),
            ),
            scoreboard,
            buf,
        );
        Widget::render(
            Paragraph::new(at_bat.matchup.format_team_lines(
                self.game.home_team.team_name,
                self.game.home_abs_challenges,
                true,
                live_details,
                &self.game.players,
            ))
            .alignment(Alignment::Right)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(border_style())
                    .padding(Padding::new(0, 0, 1, 0)),
            ),
            home,
            buf,
        );

        // only display on deck if it's the current at bat
        if is_current {
            // split into three chunks so that the center line is always exactly in the center
            let [od, split, ih] = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Length(3),
                Constraint::Fill(1),
            ])
            .areas(on_deck);

            if let (Some(on_deck), Some(in_hole)) =
                (self.game.format_on_deck(), self.game.format_in_hole())
            {
                Widget::render(
                    Paragraph::new(on_deck)
                        .alignment(Alignment::Right)
                        .block(Block::default().padding(Padding::new(2, 0, 0, 0))),
                    od,
                    buf,
                );
                Widget::render(
                    Paragraph::new(" | ".to_string())
                        .alignment(Alignment::Center)
                        .block(Block::default().padding(Padding::new(0, 0, 0, 0))),
                    split,
                    buf,
                );
                Widget::render(
                    Paragraph::new(in_hole)
                        .alignment(Alignment::Left)
                        .block(Block::default().padding(Padding::new(0, 2, 0, 0))),
                    ih,
                    buf,
                );
            }
        }
    }
}
