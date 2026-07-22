#![allow(unused, clippy::all, rust_analyzer::unresolved_method)]
// Use termimage to render the image
use crossterm::{
    event::{
        Event, EventStream, KeyCode, KeyEvent, KeyboardEnhancementFlags, MediaKeyCode,
        PushKeyboardEnhancementFlags,
    },
    execute,
};
use futures_util::StreamExt;
use lofty::{
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
};
use ratatui::{
    DefaultTerminal,
    layout::{Direction, Layout, Size},
    style::{Color, Style},
    widgets::{Block, Gauge, LineGauge, List, ListItem, ListState},
};
use ratatui::{layout::Constraint, widgets::Paragraph};
use rodio::{Decoder, Player, play};
use std::{fs::File, io::stdout, time::Duration};
use tokio::{
    sync::mpsc::{self, Receiver, Sender},
    time::Instant,
};
mod playlist;

struct UiUpdate {
    current: Duration,
    total: Duration,
    skip: bool,
}

enum PlayerCmd {
    PlayTrack(String),
    Pause,
    Resume,
    Jump(i64),
}

#[derive(PartialEq)]
enum PlayerEvent {
    Pause,
    Play,
    SkipForward,
    SkipBackward,
    HalfSkip,
}

#[derive(PartialEq)]
enum State {
    NotPlaying,
    Playing,
    Paused,
}

struct AppState {
    state: State,
    playlist: Vec<playlist::Song>,
    current_track: i32,
    last_event: PlayerEvent,
}

impl AppState {
    async fn new() -> Self {
        Self {
            state: State::NotPlaying,
            current_track: 0,
            playlist: playlist::get_songs().await,
            last_event: PlayerEvent::Pause,
        }
    }
}

fn format_duration(d: Duration) -> (u64, u64) {
    let seconds = d.as_secs();
    let off_seconds = seconds % 60;
    let minutes = seconds / 60;
    (minutes, off_seconds)
}

#[tokio::main]
async fn main() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<PlayerCmd>(32);
    let (ui_tx, ui_rx) = mpsc::channel::<UiUpdate>(32);
    tokio::task::spawn_blocking(move || {
        let mut stream = rodio::stream::DeviceSinkBuilder::open_default_sink()
            .expect("couldnt open default audio");
        stream.log_on_drop(false);
        let player = Player::connect_new(stream.mixer());

        let mut rx = cmd_rx;
        let tx = ui_tx;

        let mut total = Duration::from_secs(0);

        let mut last_update_send = Instant::now();

        let mut skip = false;

        loop {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    PlayerCmd::PlayTrack(track) => {
                        if let Ok(file) = File::open(&track) {
                            if let Ok(tagged) = Probe::open(&track).unwrap().read() {
                                total = tagged.properties().duration();
                            }
                            if let Ok(source) = Decoder::try_from(file) {
                                player.stop();
                                player.append(source);
                                player.play();
                            }
                        }
                    }
                    PlayerCmd::Pause => player.pause(),
                    PlayerCmd::Resume => player.play(),
                    PlayerCmd::Jump(seconds) => {
                        let current_pos = player.get_pos();
                        let new_pos = if seconds >= 0 {
                            current_pos + Duration::from_secs(seconds as u64)
                        } else {
                            current_pos.saturating_sub(Duration::from_secs(seconds.unsigned_abs()))
                        };
                        let _ = player.try_seek(new_pos);
                    }
                }
            }
            if rx.is_closed() {
                player.stop();
                break;
            }
            if total != Duration::from_secs(0)
                && player.get_pos() >= total - Duration::from_millis(150)
            {
                skip = true;
            }
            if last_update_send.elapsed() >= Duration::from_millis(200) {
                let skip_dupe = skip;
                skip = false;
                tx.try_send(UiUpdate {
                    current: player.get_pos(),
                    total: total,
                    skip: skip_dupe,
                })
                .unwrap();
                last_update_send = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let mut app = AppState::new().await;
    let mut terminal = ratatui::init();
    let _ = run(&mut terminal, cmd_tx, ui_rx, &mut app).await;
    ratatui::restore();
}

async fn run(
    terminal: &mut DefaultTerminal,
    cmd_tx: Sender<PlayerCmd>,
    mut ui_rx: Receiver<UiUpdate>,
    app: &mut AppState,
) -> std::io::Result<()> {
    let mut reader = EventStream::new();
    let mut ui_ticker = tokio::time::interval(Duration::from_millis(16));

    let mut current_secs = Duration::from_secs(0);
    let mut total_secs = Duration::from_secs(0);

    let reader_next = reader.next();
    loop {
        tokio::select! {
            _ = ui_ticker.tick() => {
                handle_ui(terminal, app, current_secs, total_secs);
            }
            Some(update) = ui_rx.recv() => {
                (current_secs, total_secs) = (update.current, update.total);
                if update.skip {
                    if (app.current_track + 1) < app.playlist.len() as i32 {
                        app.current_track += 1;
                        let x = app.current_track;
                        let _ = cmd_tx.send(PlayerCmd::PlayTrack(app.playlist[x as usize].path.clone())).await;
                        app.last_event = PlayerEvent::SkipForward;
                    } else {
                        app.state = State::NotPlaying;
                        app.last_event = PlayerEvent::Pause;
                    }
                }
            }
            maybe_event = reader.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_event {
                    if handle_key(key, app, cmd_tx.clone(), current_secs).await {
                        return Ok(())
                    }
                }
            }
        }
    }
}

fn handle_ui(
    terminal: &mut DefaultTerminal,
    app: &mut AppState,
    current_secs: Duration,
    total_secs: Duration,
) {
    terminal
        .draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Percentage(93), Constraint::Percentage(7)])
                .split(f.area());
            let horizontal_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![Constraint::Percentage(30), Constraint::Percentage(70)])
                .split(chunks[0]);
            let x = app.current_track;
            let current_song = &app.playlist[x as usize];
            let text = match app.last_event {
                PlayerEvent::Pause => "paused".to_string(),
                PlayerEvent::Play => "playing".to_string(),
                PlayerEvent::SkipForward => "skipped forward".to_string(),
                PlayerEvent::SkipBackward => "skipped backward".to_string(),
                PlayerEvent::HalfSkip => "replaying song".to_string(),
            };

            let (minutes, off_secs) = format_duration(current_secs);
            let (total_mins, total_off_secs) = format_duration(total_secs);

            // List
            let mut list_state =
                ListState::default().with_selected(Some(app.current_track as usize));
            let items: Vec<ListItem> = app
                .playlist
                .iter()
                .enumerate()
                .map(|(index, song)| {
                    ListItem::new(format!("{}: {} - {}", index + 1, song.title, song.artist))
                })
                .collect();
            let list = List::new(items)
                .block(Block::bordered().title("Queue"))
                .style(Style::default().fg(Color::White).bg(Color::Black))
                .highlight_style(Style::default().fg(Color::Black).bg(Color::White));
            f.render_stateful_widget(list, horizontal_split[0], &mut list_state);

            let display = Paragraph::new(text).centered();
            f.render_widget(display, horizontal_split[1]);
            let bar = Gauge::default()
                .block(Block::bordered())
                .ratio(if total_secs.as_secs() > 0 {
                    (current_secs.as_secs_f64() / total_secs.as_secs_f64()).clamp(0.0, 1.0)
                } else {
                    0.0
                })
                .label(format!(
                    "{:02}:{:02}/{:02}:{:02}",
                    minutes, off_secs, total_mins, total_off_secs
                ));
            f.render_widget(bar, chunks[1]);
        })
        .unwrap();
}

async fn handle_key(
    key: KeyEvent,
    app: &mut AppState,
    cmd_tx: Sender<PlayerCmd>,
    current_secs: Duration,
) -> bool {
    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char(' ') => match app.state {
            State::NotPlaying => {
                app.state = State::Playing;
                let x = app.current_track;
                let _ = cmd_tx
                    .send(PlayerCmd::PlayTrack(app.playlist[x as usize].path.clone()))
                    .await;
                app.last_event = PlayerEvent::Play;
            }
            State::Playing => {
                app.state = State::Paused;
                let _ = cmd_tx.send(PlayerCmd::Pause).await;
                app.last_event = PlayerEvent::Pause;
            }
            State::Paused => {
                app.state = State::Playing;
                let _ = cmd_tx.send(PlayerCmd::Resume).await;
                app.last_event = PlayerEvent::Play;
            }
        },
        KeyCode::Right => {
            let _ = cmd_tx.send(PlayerCmd::Jump(5)).await;
        }
        KeyCode::Left => {
            let _ = cmd_tx.send(PlayerCmd::Jump(-5)).await;
        }
        KeyCode::Char('j') => {
            let x = app.current_track;
            if current_secs > Duration::from_secs(3) {
                let _ = cmd_tx
                    .send(PlayerCmd::PlayTrack(app.playlist[x as usize].path.clone()))
                    .await;
                app.last_event = PlayerEvent::HalfSkip;
            } else if x > 0 {
                let flag = if app.state == State::NotPlaying || app.state == State::Paused {
                    true
                } else {
                    false
                };
                app.current_track -= 1;
                let _ = cmd_tx
                    .send(PlayerCmd::PlayTrack(
                        app.playlist[(x - 1) as usize].path.clone(),
                    ))
                    .await;
                app.last_event = PlayerEvent::SkipBackward;
                if flag {
                    let _ = cmd_tx.send(PlayerCmd::Pause).await;
                } else {
                }
            }
        }
        KeyCode::Char('k') => {
            let x = app.current_track;
            if x + 1 < app.playlist.len().try_into().unwrap() {
                let flag = if app.state == State::NotPlaying || app.state == State::Paused {
                    true
                } else {
                    false
                };
                app.current_track += 1;
                let _ = cmd_tx
                    .send(PlayerCmd::PlayTrack(
                        app.playlist[(x + 1) as usize].path.clone(),
                    ))
                    .await;
                app.last_event = PlayerEvent::SkipForward;
                if flag {
                    let _ = cmd_tx.send(PlayerCmd::Pause).await;
                }
            }
        }

        _ => {}
    }
    false
}
