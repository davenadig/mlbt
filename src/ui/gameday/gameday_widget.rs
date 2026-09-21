use crate::app::MenuItem;
use crate::components::game::reveal::linescore_through;
use crate::draw;
use crate::state::app_state::HomeOrAway;
use crate::state::boxscore::BoxscoreState;
use crate::state::gameday::GamedayState;
use crate::ui::boxscore::TeamBatterBoxscoreWidget;
use crate::ui::gameday::at_bat::AtBatWidget;
use crate::ui::gameday::matchup::MatchupWidget;
use crate::ui::gameday::plays::InningPlaysWidget;
use crate::ui::gameday::win_probability::WinProbabilityWidget;
use crate::ui::layout::LayoutAreas;
use crate::ui::linescore::LineScoreWidget;
use crate::ui::styling::{dim_style, selected_style};
use tui::prelude::{Alignment, Buffer, Line, Rect, Span, Widget};
use tui::widgets::Paragraph;

pub struct GamedayWidget<'a> {
    pub state: &'a mut GamedayState,
    pub boxscore_state: &'a mut BoxscoreState,
    pub active: HomeOrAway,
}

impl Widget for GamedayWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut panels = LayoutAreas::generate_gameday_panels(&self.state.panels, area);

        // I want the panels to be displayed [Info, Heat, Box] from left to right. So pop off
        // available panels starting with Box. Since `generate_layouts` takes into account how many
        // panels are active, all the pops are guaranteed to unwrap.
        if self.state.panels.boxscore {
            let p = panels.pop().unwrap();
            Self::draw_border(p, buf);
            let chunks = LayoutAreas::for_boxscore(p, 0);

            // Rebuilt from the plays walked to, so innings ahead of the cursor stay blank.
            // Falls back to the API's own linescore once caught up, which is authoritative.
            let walked = linescore_through(&self.state.game, self.state.cursor());
            let linescore_widget = LineScoreWidget {
                active: self.active,
                linescore: walked.as_ref().unwrap_or(&self.state.game.linescore),
            };
            Widget::render(linescore_widget, chunks[0], buf);

            match self.state.cursor() {
                // The box score is a cumulative snapshot from the API — a batter's line
                // already contains the home run you have not reached, and there is no play
                // data to rebuild it from. Saying so is better than quietly spoiling.
                Some(_) => Self::draw_suppressed_boxscore(chunks[1], buf),
                None => {
                    let boxscore_widget = TeamBatterBoxscoreWidget {
                        active: self.active,
                        state: self.boxscore_state,
                    };
                    Widget::render(boxscore_widget, chunks[1], buf);
                }
            }
        }
        if self.state.panels.at_bat {
            let p = panels.pop().unwrap();
            Self::draw_border(p, buf);
            let [matchup, at_bat] = LayoutAreas::for_at_bat(p);

            let matchup_widget = MatchupWidget {
                game: &self.state.game,
                selected_at_bat: self.state.selected_at_bat(),
                cursor: self.state.cursor(),
            };
            Widget::render(matchup_widget, matchup, buf);

            let at_bat_widget = AtBatWidget {
                game: &self.state.game,
                selected_at_bat: self.state.selected_at_bat(),
                cursor: self.state.cursor(),
                show_pitch_tracker: self.state.panels.pitch_tracker,
                show_movement: self.state.panels.movement,
            };
            Widget::render(at_bat_widget, at_bat, buf);
        }
        if self.state.panels.info {
            let p = panels.pop().unwrap();
            if self.state.scoring_plays_only {
                Self::draw_border_with_title(p, buf, " Scoring Plays ");
            } else {
                Self::draw_border(p, buf);
            };
            let chunks = LayoutAreas::for_info(p, self.state.panels.win_probability);

            let innings_widget = InningPlaysWidget {
                game: &self.state.game,
                selected_at_bat: self.state.selected_at_bat(),
                cursor: self.state.cursor(),
                scoring_only: self.state.scoring_plays_only,
                scroll_offset: &mut self.state.plays_scroll_offset,
                scroll_state: &mut self.state.plays_scroll_state,
            };
            Widget::render(innings_widget, chunks[0], buf);

            if self.state.panels.win_probability {
                let wps_widget = WinProbabilityWidget {
                    game: &self.state.game,
                    selected_at_bat: self.state.selected_at_bat(),
                    cursor: self.state.cursor(),
                    active_tab: MenuItem::Gameday,
                };
                Widget::render(wps_widget, chunks[1], buf);
            }
        }
        if self.state.listen.open {
            crate::ui::gameday::listen::render(self.state, area, buf);
        }
    }
}

impl GamedayWidget<'_> {
    fn draw_border(area: Rect, buf: &mut Buffer) {
        let block = draw::default_border();
        Widget::render(block, area, buf);
    }

    fn draw_border_with_title(area: Rect, buf: &mut Buffer, title: &str) {
        let block = draw::default_border().title(Span::styled(title, selected_style()));
        Widget::render(block, area, buf);
    }

    /// What stands in for the box score while the game is being walked.
    fn draw_suppressed_boxscore(area: Rect, buf: &mut Buffer) {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled("box score hidden", dim_style())),
            Line::from(Span::styled("it is a live whole-game total", dim_style())),
            Line::from(Span::styled("press l to catch up", dim_style())),
        ];
        Widget::render(
            Paragraph::new(lines).alignment(Alignment::Center),
            area,
            buf,
        );
    }
}
