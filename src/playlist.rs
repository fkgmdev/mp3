use std::fs;

use lofty::{file::TaggedFileExt, probe::Probe, tag::Accessor};
use tokio::fs::File;

pub struct Song {
    pub path: String,
    pub title: String,
    pub artist: String,
}

pub async fn get_songs() -> Vec<Song> {
    let songs = fs::read_to_string("playlist.txt").unwrap();
    let mut playlist: Vec<Song> = Vec::new();
    for line in songs.lines() {
        let mut artist = String::new();
        let mut title = String::new();
        let path = line.trim().clone().to_string();
        if let Ok(file) = File::open(&line).await {
            if let Ok(tagged) = Probe::open(line).unwrap().read() {
                let props = tagged.primary_tag().unwrap();
                artist = props
                    .artist()
                    .unwrap_or(std::borrow::Cow::Borrowed("Unknown Artist"))
                    .to_string();
                title = props
                    .title()
                    .unwrap_or(std::borrow::Cow::Borrowed("Unknown Title"))
                    .to_string();
            }
        }
        playlist.push(Song {
            path: path,
            title: title,
            artist: artist,
        });
    }
    playlist
}
