#![allow(unused, clippy::all)]
use crossterm::event::{Event, EventStream, KeyCode};
use futures_util::StreamExt;
use lofty::{file::AudioFile, probe::Probe};
use ratatui::DefaultTerminal;
use ratatui::widgets::Paragraph;
use rodio::{Decoder, Player, Source};
use std::{fs::File, io::BufReader, time::Duration};
use tokio::{
    sync::mpsc::{self, Receiver, Sender},
    time::Instant,
};

enum PlayerCmd {
    PlayTrack(String),
    Pause,
    Resume,
    Jump(i64),
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
    let (ui_tx, ui_rx) = mpsc::channel::<(Duration, Duration)>(32);
    tokio::task::spawn_blocking(move || {
        let mut stream = rodio::stream::DeviceSinkBuilder::open_default_sink()
            .expect("couldnt open default audio");
        stream.log_on_drop(false);
        let player = Player::connect_new(stream.mixer());

        let mut rx = cmd_rx;
        let tx = ui_tx;

        let mut total = Duration::from_secs(0);

        let mut last_update_send = Instant::now();

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
            if last_update_send.elapsed() >= Duration::from_millis(200) {
                tx.try_send((player.get_pos(), total));
                last_update_send = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let mut app = AppState::new();
    let mut terminal = ratatui::init();
    let _ = run(&mut terminal, cmd_tx, ui_rx, &mut app).await;
    ratatui::restore();
}

async fn run(
    terminal: &mut DefaultTerminal,
    cmd_tx: Sender<PlayerCmd>,
    mut ui_rx: Receiver<(Duration, Duration)>,
    app: &mut AppState,
) -> std::io::Result<()> {
    let mut reader = EventStream::new();
    let mut ui_ticker = tokio::time::interval(Duration::from_millis(16));

    let mut current_secs = Duration::from_secs(0);
    let mut total_secs = Duration::from_secs(0);

    loop {
        tokio::select! {
            _ = ui_ticker.tick() => {
                terminal.draw(|f| {
                    let mut text = match app.state {
                        State::NotPlaying => "Idle".to_string(),
                        State::Playing => "Playing".to_string(),
                        State::Paused => "Paused".to_string(),
                    };
                    text = text + format!(" {:?}/{:?}", current_secs, total_secs).as_str();
                    let display = Paragraph::new(text);
                    f.render_widget(display, f.area());
                });
            }
            Some((current, total)) = ui_rx.recv() => {
                (current_secs, total_secs) = (current, total);
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
                        KeyCode::Right => {
                            let _ = cmd_tx.send(PlayerCmd::Jump(5)).await;
                        }
                        KeyCode::Left => {
                            let _ = cmd_tx.send(PlayerCmd::Jump(-5)).await;
                        }

                        _ => {}
                    }
                }
            }
        }
    }
}
