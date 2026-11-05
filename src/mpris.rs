use std::sync::Arc;

use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player as Server, Time, TrackId};
use tokio::sync::mpsc;
use tokio::task::LocalSet;

use snellar_core::model::{Repeat, Track};
use snellar_core::product;

pub enum Update {
    Track(Option<Track>),
    Playing(bool),
    Position(f64),
    Seeked(f64),
    Volume(f64),
    Modes { shuffle: bool, repeat: Repeat },
}

pub enum Command {
    Toggle,
    Next,
    Prev,
    Seek(f64),
    Volume(f64),
    Shuffle(bool),
    Repeat(Repeat),
}

pub type Control = Arc<dyn Fn(Command) + Send + Sync>;

pub fn spawn(control: Control) -> mpsc::UnboundedSender<Update> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name(format!("{}-mpris", product::ID))
        .spawn(move || {
            let local = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("mpris runtime");
            if let Err(e) = local.block_on(LocalSet::new().run_until(serve(control, rx))) {
                snellar_core::journal::error("mpris", e.to_string());
            }
        })
        .expect("mpris thread");
    tx
}

fn track_id(t: &Track) -> TrackId {
    let id: String = t.id.as_str().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    TrackId::try_from(format!("{}/{id}", product::MPRIS_TRACK)).unwrap_or(TrackId::NO_TRACK)
}

fn metadata(t: &Track) -> Metadata {
    let mut m = Metadata::builder().trackid(track_id(t)).title(t.title.clone()).artist(t.artists.iter().map(|a| a.name.clone())).build();
    m.set_album(t.album.as_ref().map(|a| a.name.clone()));
    m.set_art_url(Some(t.thumb.clone()).filter(|u| !u.is_empty()));
    m.set_length((t.duration > 0.0).then(|| Time::from_micros((t.duration * 1e6) as i64)));
    m
}

fn micros(secs: f64) -> Time {
    Time::from_micros((secs * 1e6) as i64)
}

async fn serve(control: Control, mut rx: mpsc::UnboundedReceiver<Update>) -> mpris_server::zbus::Result<()> {
    let mp = Server::builder(product::ID)
        .identity(product::NAME)
        .desktop_entry(product::ID)
        .can_play(true)
        .can_pause(true)
        .can_seek(true)
        .build()
        .await?;

    let c = control.clone();
    mp.connect_play_pause(move |_| c(Command::Toggle));
    let c = control.clone();
    mp.connect_play(move |m| {
        if m.playback_status() != PlaybackStatus::Playing {
            c(Command::Toggle);
        }
    });
    let c = control.clone();
    mp.connect_pause(move |m| {
        if m.playback_status() == PlaybackStatus::Playing {
            c(Command::Toggle);
        }
    });
    let c = control.clone();
    mp.connect_next(move |_| c(Command::Next));
    let c = control.clone();
    mp.connect_previous(move |_| c(Command::Prev));
    let c = control.clone();
    mp.connect_seek(move |m, offset| {
        let target = (m.position().as_micros() + offset.as_micros()).max(0) as f64 / 1e6;
        c(Command::Seek(target));
    });
    let c = control.clone();
    mp.connect_set_position(move |m, id, pos| {
        if m.metadata().trackid().as_ref() == Some(id) {
            c(Command::Seek(pos.as_micros() as f64 / 1e6));
        }
    });
    let c = control.clone();
    mp.connect_set_volume(move |_, v| c(Command::Volume(v * 100.0)));
    let c = control.clone();
    mp.connect_set_shuffle(move |_, on| c(Command::Shuffle(on)));
    mp.connect_set_loop_status(move |_, status| {
        control(Command::Repeat(match status {
            LoopStatus::None => Repeat::Off,
            LoopStatus::Track => Repeat::One,
            LoopStatus::Playlist => Repeat::All,
        }))
    });

    tokio::task::spawn_local(mp.run());

    let mut has_track = false;
    while let Some(update) = rx.recv().await {
        match update {
            Update::Track(t) => {
                has_track = t.is_some();
                mp.set_metadata(t.as_ref().map_or_else(Metadata::new, metadata)).await?;
                mp.set_can_go_next(has_track).await?;
                mp.set_can_go_previous(has_track).await?;
                if !has_track {
                    mp.set_playback_status(PlaybackStatus::Stopped).await?;
                }
            }
            Update::Playing(on) if has_track => {
                mp.set_playback_status(if on { PlaybackStatus::Playing } else { PlaybackStatus::Paused }).await?;
            }
            Update::Playing(_) => {}
            Update::Position(s) => mp.set_position(micros(s)),
            Update::Seeked(s) => {
                mp.set_position(micros(s));
                mp.seeked(micros(s)).await?;
            }
            Update::Volume(v) => mp.set_volume(v).await?,
            Update::Modes { shuffle, repeat } => {
                mp.set_shuffle(shuffle).await?;
                mp.set_loop_status(match repeat {
                    Repeat::Off => LoopStatus::None,
                    Repeat::One => LoopStatus::Track,
                    Repeat::All => LoopStatus::Playlist,
                })
                .await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use snellar_core::model::Track;

    use super::*;

    #[test]
    fn the_length_reaches_mpris_as_soon_as_the_track_has_one() {
        let track = Track { id: "a".into(), title: "T".into(), ..Track::default() };
        assert_eq!(metadata(&track).length(), None);
        let timed = Track { duration: 181.8, ..track };
        assert_eq!(metadata(&timed).length(), Some(Time::from_micros(181_800_000)));
    }
}
