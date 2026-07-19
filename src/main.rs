#![allow(unused, clippy::all)]
use crossterm::event::{Event, EventStream, KeyCode};
use futures_util::StreamExt;
use ratatui::DefaultTerminal;
use ratatui::widgets::Paragraph;
use rodio::{Decoder, Player};
use std::{fs::File, io::BufReader, time::Duration};
use tokio::sync::mpsc::{self, Sender};

enum PlayerCmd {
    PlayTrack(String),
    Pause,
    Resume,
}

enum State {
    NotPlaying,
    Playing,
    Paused,
}

struct AppState {
    state: State,
    current_track: String,
}

impl AppState {
    fn new() -> Self {
        Self {
            state: State::NotPlaying,
            current_track: String::new(),
        }
    }
}

#[tokio::main]
async fn main() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<PlayerCmd>(32);
    tokio::task::spawn_blocking(move || {
        let stream = rodio::stream::DeviceSinkBuilder::open_default_sink()
            .expect("couldnt open default audio");
        let player = Player::connect_new(stream.mixer());

        let mut rx = cmd_rx;

        while let Some(cmd) = rx.blocking_recv() {
            match cmd {
                PlayerCmd::PlayTrack(track) => {
                    if let Ok(file) = File::open(track) {
                        if let Ok(source) = Decoder::try_from(BufReader::new(file)) {
                            player.stop();
                            player.append(source);
                            player.play();
                        }
                    }
                }
                PlayerCmd::Pause => player.pause(),
                PlayerCmd::Resume => player.play(),
            }
        }
    });
    let mut app = AppState::new();
    let mut terminal = ratatui::init();
    let _ = run(&mut terminal, cmd_tx, &mut app).await;
    ratatui::restore();
}

async fn run(
    terminal: &mut DefaultTerminal,
    cmd_tx: Sender<PlayerCmd>,
    app: &mut AppState,
) -> std::io::Result<()> {
    let mut reader = EventStream::new();
    let mut ui_ticker = tokio::time::interval(Duration::from_millis(16));
    loop {
        tokio::select! {
            _ = ui_ticker.tick() => {
                terminal.draw(|f| {
                    let text = match app.state {
                        State::NotPlaying => "Idle",
                        State::Playing => "Playing",
                        State::Paused => "Paused",
                    };
                    let display = Paragraph::new(text);
                    f.render_widget(display, f.area());
                });
            }
            maybe_event = reader.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_event {
                    match key.code {
                        KeyCode::Char('q') => {
                            return Ok(())
                        }
                        KeyCode::Char(' ') => {
                            match app.state {
                                State::NotPlaying => {
                                    app.state = State::Playing;
                                    app.current_track = String::from("test.mp3");
                                    let _ = cmd_tx.send(PlayerCmd::PlayTrack(app.current_track.clone())).await;
                                }
                                State::Playing => {
                                    app.state = State::Paused;
                                    let _ = cmd_tx.send(PlayerCmd::Pause).await;
                                }
                                State::Paused => {
                                    app.state = State::Playing;
                                    let _ = cmd_tx.send(PlayerCmd::Resume).await;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

