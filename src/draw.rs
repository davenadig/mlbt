use tui::backend::Backend;
use tui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use tui::style::{Color, Modifier, Style};
use tui::text::{Line, Span};
use tui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Tabs, Widget};
use tui::{Frame, Terminal};

use crate::app::{App, DebugState, MenuItem};
use crate::components::debug::DebugInfo;
use crate::components::stats::table::TeamOrPlayer;
use crate::state::network::{ERROR_CHAR, LoadingState};
use crate::state::settings_editor::SettingsFocus;
use crate::state::stats::ActivePane;
use crate::ui::boxscore::TeamBatterBoxscoreWidget;
use crate::ui::date_selector::DateSelectorWidget;
use crate::ui::decision_pitchers::DecisionPitchersWidget;
use crate::ui::gameday::gameday_widget::GamedayWidget;
use crate::ui::gameday::win_probability::WinProbabilityWidget;
use crate::ui::help::help_widget::HelpWidget;
use crate::ui::input_popup::{InputPopup, popup_cursor_position};
use crate::ui::layout::LayoutAreas;
use crate::ui::linescore::LineScoreWidget;
use crate::ui::logs::LogWidget;
use crate::ui::player_profile::PlayerProfileWidget;
use crate::ui::probable_pitchers::ProbablePitchersWidget;
use crate::ui::schedule::ScheduleWidget;
use crate::ui::standings::StandingsWidget;
use crate::ui::stats::{STATS_OPTIONS_WIDTH, StatsDataWidget, StatsOptionsWidget};
use crate::ui::styling::{TEXT_COLOR, border_style};
use crate::ui::team_page::TeamPageWidget;

static TABS: &[&str; 4] = &["Scoreboard", "Gameday", "Stats", "Standings"];

pub fn draw<B>(terminal: &mut Terminal<B>, app: &mut App, is_loading: LoadingState)
where
    B: Backend,
{
    let current_size = terminal.size().unwrap_or_default();
    let mut main_layout = LayoutAreas::new(current_size);

    if current_size.width <= 10 || current_size.height <= 10 {
        return;
    }

    terminal
        .draw(|f| {
            main_layout.update(f.area(), app.settings.full_screen);

            if !app.settings.full_screen {
                draw_tabs(f, main_layout.top_bar, app);
            }

            match app.state.active_tab {
                MenuItem::Scoreboard => draw_scoreboard(f, main_layout.main, app),
                MenuItem::DatePicker => {
                    match app.state.previous_tab {
                        MenuItem::Scoreboard => draw_scoreboard(f, main_layout.main, app),
                        MenuItem::Standings => draw_standings(f, main_layout.main, app),
                        MenuItem::Stats => draw_stats(f, main_layout.main, app),
                        _ => (),
                    }
                    draw_date_picker(f, main_layout.main, app);
                }
                MenuItem::Gameday => draw_gameday(f, main_layout.main, app),
                MenuItem::Stats => draw_stats(f, main_layout.main, app),
                MenuItem::Standings => draw_standings(f, main_layout.main, app),
                MenuItem::Help => {
                    let area = f.area();
                    let help_area = Rect::new(
                        area.x,
                        area.y,
                        area.width,
                        area.height.saturating_sub(main_layout.footer.height),
                    );
                    draw_help(f, help_area, app);
                }
            }
            if app.state.debug_state == DebugState::On {
                let mut dbi = DebugInfo::new();
                dbi.gather_info(f, app);
                dbi.render(f, main_layout.main, app.state.show_logs);
            }

            draw_loading_spinner(f, f.area(), app, is_loading);
            draw_key_hints(f, main_layout.footer, app);
        })
        .unwrap();
}

pub fn default_border<'a>() -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style())
}

fn draw_border(f: &mut Frame, rect: Rect) {
    let block = default_border();
    f.render_widget(block, rect);
}

fn draw_loading_spinner(f: &mut Frame, area: Rect, app: &App, loading: LoadingState) {
    if !loading.is_loading && loading.spinner_char != ERROR_CHAR {
        return;
    }

    let style = match loading.spinner_char {
        ERROR_CHAR => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::default()),
    };

    let spinner = Paragraph::new(loading.spinner_char.to_string())
        .alignment(Alignment::Right)
        .style(style);

    let area = if app.settings.full_screen || app.state.active_tab == MenuItem::Help {
        // render in the bottom right
        Rect::new(
            area.width.saturating_sub(3),
            area.height.saturating_sub(2),
            1,
            1,
        )
    } else {
        // render in the top right
        Rect::new(area.width.saturating_sub(11), 1, 1, 1)
    };
    f.render_widget(spinner, area);
}

fn draw_tabs(f: &mut Frame, top_bar: Rect, app: &App) {
    let border_style = border_style();
    let border_type = BorderType::Rounded;

    let titles: Vec<Line> = TABS.iter().map(|t| Line::from(*t)).collect();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(border_style),
        )
        .highlight_style(
            // underline the active tab
            Style::default().add_modifier(Modifier::UNDERLINED),
        )
        .select(app.state.active_tab as usize)
        .style(TEXT_COLOR);
    f.render_widget(tabs, top_bar);
}

type KeyHint = (&'static str, &'static str);
const KEY_HINT_BG: Color = Color::Rgb(24, 24, 27);
const KEY_HINT_KEY: Color = Color::Rgb(250, 204, 21);
const KEY_HINT_TEXT: Color = Color::Rgb(250, 250, 250);

fn team_page_key_hints(has_player_profile: bool) -> &'static [KeyHint] {
    if has_player_profile {
        &[
            ("j/k", "scroll"),
            ("J/K", "page"),
            ("s", "season"),
            ("Esc", "back"),
        ]
    } else {
        &[
            ("j/k", "move"),
            ("J/K", "page"),
            ("←/→/Tab", "section"),
            ("c", "calendar"),
            ("r", "roster"),
            ("Enter", "player"),
            ("Esc", "back"),
        ]
    }
}

fn context_key_hints(app: &App) -> &'static [KeyHint] {
    match app.state.active_tab {
        MenuItem::Scoreboard => &[
            ("j/k", "game"),
            ("J/K", "box"),
            ("a/h", "lineup"),
            ("Enter", "gameday"),
            ("c", "card"),
            ("w", "win%"),
            ("s", "sort"),
            (":", "date"),
        ],
        MenuItem::Gameday if app.state.gameday.listen.open => &[
            ("←/→/Tab", "team"),
            ("Enter", "tune"),
            ("x", "stop"),
            ("Esc", "close"),
        ],
        // Sonos ads on: the footer leads with the ad keys, since that is what the couch needs.
        MenuItem::Gameday if app.state.gameday.ads.active() => &[
            ("A", "Sonos off"),
            (".", "delay +2s"),
            (",", "delay -2s"),
            ("m/u", "mute/unmute"),
            ("←/→", "pitch"),
            ("l", "live"),
            ("s", "start"),
            ("!", "scoring"),
            ("c", "card"),
        ],
        MenuItem::Gameday => &[
            ("A", "Sonos"),
            ("L", "Listen"),
            ("←/→", "pitch"),
            ("J/K", "box"),
            ("s", "start"),
            ("l", "live"),
            ("i", "info"),
            ("p", "pitches"),
            ("b", "box"),
            ("w", "win%"),
            ("n", "pitches"),
            ("v", "spin"),
            ("!", "scoring"),
            ("a/h", "lineup"),
            ("c", "card"),
        ],
        MenuItem::Stats if app.state.stats.search.is_open => &[
            ("type", "query"),
            ("↑/↓", "result"),
            ("Backspace", "delete"),
            ("Enter", "search"),
            ("Esc", "cancel"),
        ],
        MenuItem::Stats if app.state.stats.team_page.is_some() => {
            let team_page = app.state.stats.team_page.as_ref().unwrap();
            team_page_key_hints(team_page.has_player_profile())
        }
        MenuItem::Stats if app.state.stats.has_player_profile() => team_page_key_hints(true),
        MenuItem::Stats => match (
            app.state.stats.active_pane,
            app.state.stats.stat_type.team_player,
        ) {
            (ActivePane::Data, TeamOrPlayer::Player) => &[
                ("j/k", "move"),
                ("J/K", "page"),
                ("Tab", "pane"),
                ("h/p", "hit/pitch"),
                ("t/l", "team/player"),
                ("a/u", "all/qual"),
                ("s", "sort"),
                ("o", "options"),
                ("Enter", "open"),
                ("^f", "search"),
                (":", "date"),
            ],
            (ActivePane::Data, TeamOrPlayer::Team) => &[
                ("j/k", "move"),
                ("J/K", "page"),
                ("Tab", "pane"),
                ("h/p", "hit/pitch"),
                ("t/l", "team/player"),
                ("s", "sort"),
                ("o", "options"),
                ("Enter", "open"),
                ("^f", "search"),
                (":", "date"),
            ],
            (ActivePane::Options, TeamOrPlayer::Player) => &[
                ("j/k", "move"),
                ("Tab", "data"),
                ("h/p", "hit/pitch"),
                ("t/l", "team/player"),
                ("a/u", "all/qual"),
                ("Enter", "toggle"),
                ("o", "close"),
                ("^f", "search"),
                (":", "date"),
            ],
            (ActivePane::Options, TeamOrPlayer::Team) => &[
                ("j/k", "move"),
                ("Tab", "data"),
                ("h/p", "hit/pitch"),
                ("t/l", "team/player"),
                ("Enter", "toggle"),
                ("o", "close"),
                ("^f", "search"),
                (":", "date"),
            ],
        },
        MenuItem::Standings if app.state.standings.team_page.is_some() => {
            let team_page = app.state.standings.team_page.as_ref().unwrap();
            team_page_key_hints(team_page.has_player_profile())
        }
        MenuItem::Standings => &[
            ("j/k", "team"),
            ("Enter", "open"),
            ("l", "league"),
            (":", "date"),
        ],
        MenuItem::DatePicker => &[
            ("type", "date"),
            ("←/→", "day"),
            ("Backspace", "delete"),
            ("Enter", "apply"),
            ("Esc", "cancel"),
        ],
        MenuItem::Help if app.state.settings_editor.picker.is_some() => &[
            ("j/k", "option"),
            ("A-Z", "jump"),
            ("Enter", "apply"),
            ("Esc", "cancel"),
        ],
        MenuItem::Help if app.state.settings_editor.focus == SettingsFocus::Settings => &[
            ("j/k", "field"),
            ("Tab", "docs"),
            ("Enter", "change"),
            ("Esc", "close"),
        ],
        MenuItem::Help => &[
            ("j/k", "scroll"),
            ("J/K", "page"),
            ("Tab", "settings"),
            ("Esc", "close"),
        ],
    }
}

fn navigation_key_hints(app: &App) -> &'static [KeyHint] {
    if app.state.active_tab == MenuItem::Gameday && app.state.gameday.listen.open {
        return &[];
    }
    if app.state.active_tab == MenuItem::DatePicker
        || (app.state.active_tab == MenuItem::Stats && app.state.stats.search.is_open)
        || (app.state.active_tab == MenuItem::Help && app.state.settings_editor.picker.is_some())
    {
        &[]
    } else {
        &[
            ("1", "scoreboard"),
            ("2", "gameday"),
            ("3", "stats"),
            ("4", "standings"),
            ("f", "full"),
        ]
    }
}

fn global_key_hints(app: &App) -> &'static [KeyHint] {
    if app.state.active_tab == MenuItem::Gameday && app.state.gameday.listen.open {
        return &[("q", "quit")];
    }
    if app.state.active_tab == MenuItem::DatePicker
        || (app.state.active_tab == MenuItem::Stats && app.state.stats.search.is_open)
    {
        &[]
    } else if app.state.active_tab == MenuItem::Help {
        &[("q", "quit")]
    } else {
        &[("?", "help"), ("q", "quit")]
    }
}

fn hint_spans(hints: &[KeyHint]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (index, (key, label)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" ", Style::default().bg(KEY_HINT_BG)));
        }
        spans.push(Span::styled(
            *key,
            Style::default()
                .fg(KEY_HINT_KEY)
                .bg(KEY_HINT_BG)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(KEY_HINT_TEXT).bg(KEY_HINT_BG),
        ));
    }
    spans
}

fn draw_key_hints(f: &mut Frame, area: Rect, app: &App) {
    let [context_area, navigation_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    let global_width = global_key_hints(app)
        .iter()
        .map(|(key, label)| key.chars().count() + label.chars().count() + 1)
        .sum::<usize>()
        + global_key_hints(app).len().saturating_sub(1);
    let [navigation_area, global_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(global_width as u16)])
            .areas(navigation_area);

    f.render_widget(
        Block::default().style(Style::default().bg(KEY_HINT_BG)),
        area,
    );
    f.render_widget(
        Paragraph::new(Line::from(hint_spans(context_key_hints(app))))
            .style(Style::default().bg(KEY_HINT_BG)),
        context_area,
    );
    f.render_widget(
        Paragraph::new(Line::from(hint_spans(navigation_key_hints(app))))
            .style(Style::default().bg(KEY_HINT_BG)),
        navigation_area,
    );
    f.render_widget(
        Paragraph::new(Line::from(hint_spans(global_key_hints(app))))
            .alignment(Alignment::Right)
            .style(Style::default().bg(KEY_HINT_BG)),
        global_area,
    );
}

fn draw_scoreboard(f: &mut Frame, rect: Rect, app: &mut App) {
    // TODO calculate width based on table sizes
    let direction = match f.area().width {
        w if w < 125 => Direction::Vertical,
        _ => Direction::Horizontal,
    };
    let [scoreboard, boxscore] = Layout::default()
        .direction(direction)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .areas(rect);

    // display scores on left side
    f.render_stateful_widget(
        ScheduleWidget {
            tz_abbreviation: app.settings.timezone_abbreviation.clone(),
        },
        scoreboard,
        &mut app.state.schedule,
    );
    if app.state.schedule.show_win_probability {
        draw_win_probability(f, scoreboard, app);
    }

    // display probable pitchers or line score and box score on right
    if let Some(matchup) = app.state.schedule.get_probable_pitchers_opt() {
        f.render_widget(ProbablePitchersWidget { matchup }, boxscore);
    } else {
        draw_border(f, boxscore);
        draw_linescore_boxscore(f, boxscore, app);
    }
}

fn draw_linescore_boxscore(f: &mut Frame, rect: Rect, app: &mut App) {
    // `is_final` only returns true once the live api has populated GameState, which keeps
    // decisions in sync with the linescore and box score instead of leading them on scroll.
    let decision_pitchers = if app.state.gameday.is_final() {
        app.state
            .schedule
            .get_decision_pitchers_for_game(app.state.gameday.current_game_id())
    } else {
        None
    };
    let pitcher_count = decision_pitchers.map_or(0, |d| d.count());
    let chunks = LayoutAreas::for_boxscore(rect, pitcher_count);

    f.render_widget(
        LineScoreWidget {
            active: app.state.box_score.active_team,
            linescore: &app.state.gameday.game.linescore,
        },
        chunks[0],
    );
    let boxscore_chunk = if let Some(decisions) = decision_pitchers {
        f.render_widget(DecisionPitchersWidget { decisions }, chunks[1]);
        chunks[2]
    } else {
        chunks[1]
    };
    f.render_widget(
        TeamBatterBoxscoreWidget {
            active: app.state.box_score.active_team,
            state: &mut app.state.box_score,
        },
        boxscore_chunk,
    );
}

fn draw_date_picker(f: &mut Frame, rect: Rect, app: &mut App) {
    let chunk = LayoutAreas::create_date_picker(rect);
    f.render_stateful_widget(DateSelectorWidget {}, chunk, &mut app.state.date_input);

    let (cx, cy) = popup_cursor_position(chunk, app.state.date_input.text.len() as u16);
    f.set_cursor_position((cx, cy));
}

fn draw_gameday(f: &mut Frame, rect: Rect, app: &mut App) {
    let [status, controls, game] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(rect);
    let ads = &app.state.gameday.ads;
    f.render_widget(
        Paragraph::new(ads.label()).style(Style::default().fg(KEY_HINT_KEY).bg(KEY_HINT_BG)),
        status,
    );
    let help = if ads.status.message.is_empty() {
        "A Sonos on/off   . delay +2s   , delay -2s   m mute   u unmute   (stops when leaving Gameday)"
            .to_string()
    } else {
        format!("A on/off  ./, delay  m/u mute/unmute | {}", ads.status.message)
    };
    f.render_widget(
        Paragraph::new(help).style(Style::default().bg(KEY_HINT_BG)),
        controls,
    );
    f.render_widget(
        GamedayWidget {
            active: app.state.box_score.active_team,
            state: &mut app.state.gameday,
            boxscore_state: &mut app.state.box_score,
        },
        game,
    );
}

fn draw_stats(f: &mut Frame, rect: Rect, app: &mut App) {
    if let Some(tp) = &mut app.state.stats.team_page {
        if let Some(profile) = &mut tp.player_profile {
            PlayerProfileWidget { state: profile }.render(rect, f.buffer_mut());
            return;
        }
        TeamPageWidget { state: tp }.render(rect, f.buffer_mut());
        return;
    }
    if let Some(profile) = &mut app.state.stats.player_profile {
        PlayerProfileWidget { state: profile }.render(rect, f.buffer_mut());
        return;
    }

    // Split horizontally first: data pane (left) and options pane (right)
    let (data_area, options_area) =
        if app.state.stats.show_options && rect.width > STATS_OPTIONS_WIDTH {
            let [data, options] = Layout::horizontal([
                Constraint::Length(rect.width - STATS_OPTIONS_WIDTH),
                Constraint::Length(STATS_OPTIONS_WIDTH),
            ])
            .areas(rect);
            (data, Some(options))
        } else {
            (rect, None)
        };

    // If search is open, shrink only the data pane to make room for the search bar
    let (data_table_area, search_area) = if app.state.stats.search.is_open {
        let [table, search] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(4)]).areas(data_area);
        (table, Some(search))
    } else {
        (data_area, None)
    };

    // TODO by taking into account the width of the options pane I'm basically removing that amount
    // of space for columns. If I didn't, you could select columns that would be covered by the
    // options pane, but then when its disabled would become visible.
    app.state.stats.table.trim_columns(data_table_area.width);
    f.render_stateful_widget(StatsDataWidget {}, data_table_area, &mut app.state.stats);

    if let Some(options_area) = options_area {
        f.render_stateful_widget(StatsOptionsWidget {}, options_area, &mut app.state.stats);
    }

    if let Some(search_area) = search_area {
        let title = format!("Search {}", app.state.stats.stat_type.search_label());
        let total = app.state.stats.total_row_count();
        let filtered = app.state.stats.search.matched_indices.len();
        let info = if app.state.stats.search.is_filtering() {
            format!("{}/{}", filtered, total)
        } else {
            format!("{}", total)
        };
        InputPopup {
            title: &title,
            instructions: "Press Enter to search or Esc to cancel",
            input_text: &app.state.stats.search.input,
            border_color: Color::Blue,
            info: Some(&info),
        }
        .render(search_area, f.buffer_mut());

        let (cx, cy) =
            popup_cursor_position(search_area, app.state.stats.search.input.len() as u16);
        f.set_cursor_position((cx, cy));
    }
}

fn draw_standings(f: &mut Frame, rect: Rect, app: &mut App) {
    if let Some(tp) = &mut app.state.standings.team_page {
        if let Some(profile) = &mut tp.player_profile {
            PlayerProfileWidget { state: profile }.render(rect, f.buffer_mut());
            return;
        }
        TeamPageWidget { state: tp }.render(rect, f.buffer_mut());
        return;
    }
    f.render_stateful_widget(StandingsWidget {}, rect, &mut app.state.standings);
}

fn draw_win_probability(f: &mut Frame, rect: Rect, app: &mut App) {
    // only render if it doesn't overlap the schedule
    let minimum_size =
        WinProbabilityWidget::get_min_table_height() + app.state.schedule.schedule.len() + 2; // +2 for borders 
    if rect.height > minimum_size as u16 {
        f.render_widget(
            WinProbabilityWidget {
                cursor: None,
                game: &app.state.gameday.game,
                selected_at_bat: app.state.gameday.selected_at_bat(),
                active_tab: MenuItem::Scoreboard,
            },
            rect,
        );
    }
}

fn draw_help(f: &mut Frame, rect: Rect, app: &mut App) {
    f.render_widget(Clear, rect);

    if app.state.show_logs {
        draw_border(f, rect);
        f.render_widget(LogWidget {}, rect);
        return;
    }

    draw_border(f, rect);
    f.render_stateful_widget(
        HelpWidget {
            // use previous tab because help has been set to active at this point
            active_tab: app.state.previous_tab,
            settings: &app.settings,
            editor: &app.state.settings_editor,
        },
        rect,
        &mut app.state.help.state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gameday_renders_sonos_delay_and_separate_controls() {
        use tui::backend::TestBackend;
        for width in [80, 120] {
            let mut app = App::new();
            app.state.active_tab = MenuItem::Gameday;
            app.state.gameday.ads.status.lag = Some(11.2);
            app.state.gameday.ads.status.mute_margin = Some(10.0);
            app.state.gameday.ads.status.unmute_margin = Some(10.0);
            let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
            terminal
                .draw(|f| draw_gameday(f, f.area(), &mut app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let lines: Vec<String> = (0..2)
                .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
                .collect();
            assert!(lines[0].contains("Sonos ads OFF | Delay 11.2s"));
            assert!(lines[0].contains("Mute +10s / Unmute -10s"));
            assert!(lines[1].contains(". delay +2s"));
            assert!(lines[1].contains(", delay -2s"));
            assert!(lines[1].contains("m mute   u unmute"));
        }
    }

    #[test]
    fn scoreboard_footer_surfaces_lineup_controls() {
        let app = App::new();
        let hints = context_key_hints(&app);
        assert!(hints.contains(&("a/h", "lineup")));
        assert!(hints.contains(&("Enter", "gameday")));
        assert!(hints.contains(&("c", "card")));
    }

    #[test]
    fn gameday_footer_surfaces_sonos_keys_and_leads_with_them_when_active() {
        let mut app = App::new();
        app.state.active_tab = MenuItem::Gameday;
        assert!(context_key_hints(&app).contains(&("A", "Sonos")));
        assert!(!context_key_hints(&app).iter().any(|(k, _)| *k == "."));
        let mut cmd = std::process::Command::new("python3");
        cmd.args(["-c", "import sys\nfor _ in sys.stdin: pass"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        app.state.gameday.ads.launch(cmd, 111).unwrap();
        let hints = context_key_hints(&app);
        assert_eq!(hints[0], ("A", "Sonos off"));
        assert!(hints.contains(&(".", "delay +2s")));
        assert!(hints.contains(&(",", "delay -2s")));
        assert!(hints.contains(&("m/u", "mute/unmute")));
        let width: usize = hints.iter().map(|(k, l)| k.len() + l.len() + 2).sum();
        assert!(width <= 100, "footer must fit a 100-column pane: {width}");
    }

    #[test]
    fn stats_footer_surfaces_mode_and_pane_controls() {
        let mut app = App::new();
        app.state.active_tab = MenuItem::Stats;
        app.state.stats.active_pane = ActivePane::Data;
        app.state.stats.stat_type.team_player = TeamOrPlayer::Player;

        let hints = context_key_hints(&app);
        assert!(hints.contains(&("h/p", "hit/pitch")));
        assert!(hints.contains(&("t/l", "team/player")));
        assert!(hints.contains(&("a/u", "all/qual")));
        assert!(hints.contains(&("Tab", "pane")));
        assert!(hints.contains(&("Enter", "open")));
    }

    #[test]
    fn regular_views_surface_navigation_help_and_quit() {
        let mut app = App::new();
        for tab in [
            MenuItem::Scoreboard,
            MenuItem::Gameday,
            MenuItem::Stats,
            MenuItem::Standings,
        ] {
            app.state.active_tab = tab;
            assert!(navigation_key_hints(&app).contains(&("f", "full")));
            assert!(global_key_hints(&app).contains(&("?", "help")));
            assert!(global_key_hints(&app).contains(&("q", "quit")));
        }
    }

    #[test]
    fn text_entry_footers_do_not_advertise_captured_global_keys() {
        let mut app = App::new();
        app.state.active_tab = MenuItem::DatePicker;
        assert!(navigation_key_hints(&app).is_empty());
        assert!(global_key_hints(&app).is_empty());

        app.state.active_tab = MenuItem::Stats;
        app.state.stats.open_search();
        assert!(navigation_key_hints(&app).is_empty());
        assert!(global_key_hints(&app).is_empty());
    }
}
