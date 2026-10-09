//! Spark Engine - Audio: kira backend for `spark_core::audio`.
//!
//! Supports wav, ogg, mp3 and flac. Sound files are decoded once and cached per path.
//!
//! ```ignore
//! if let Some(audio) = spark_audio::KiraBackend::new() {
//!     world.audio.set_backend(Box::new(audio));
//! }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kira::sound::PlaybackState;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Panning, PlaybackRate, Tween};
use spark_core::{AudioBackend, AudioCommand, BusId, PlayParams, SoundId};

/// Linear volume (0..) to decibels.
fn db(volume: f32) -> Decibels {
    if volume <= 0.001 { Decibels::SILENCE } else { Decibels((20.0 * volume.log10()).max(-60.0)) }
}

fn tween(seconds: f32) -> Tween {
    Tween { duration: Duration::from_secs_f32(seconds.max(0.0)), ..Default::default() }
}

/// Plays sounds through the default output device.
pub struct KiraBackend {
    manager: AudioManager<DefaultBackend>,
    cache: HashMap<PathBuf, Option<StaticSoundData>>,
    sounds: HashMap<SoundId, (StaticSoundHandle, Option<BusId>)>,
    /// One kira sub-track per bus, created on first use.
    buses: HashMap<BusId, TrackHandle>,
}

impl KiraBackend {
    /// Opens the default audio device. `None` (with a warning) if there is no usable device.
    pub fn new() -> Option<Self> {
        match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(manager) => Some(Self { manager, cache: HashMap::new(), sounds: HashMap::new(), buses: HashMap::new() }),
            Err(e) => {
                log::warn!("audio disabled: {e}");
                None
            }
        }
    }

    fn load(&mut self, path: &Path) -> Option<StaticSoundData> {
        self.cache
            .entry(path.to_path_buf())
            .or_insert_with(|| match spark_core::vfs::read(path).map_err(|e| e.to_string()).and_then(|b| {
                StaticSoundData::from_cursor(std::io::Cursor::new(b)).map_err(|e| e.to_string())
            }) {
                Ok(data) => Some(data),
                Err(e) => {
                    log::error!("can't load sound {}: {e}", path.display());
                    None
                }
            })
            .clone()
    }

    fn play(&mut self, id: SoundId, path: &Path, p: PlayParams) {
        let Some(data) = self.load(path) else { return };
        let mut data = data
            .volume(db(p.volume))
            .playback_rate(PlaybackRate(p.pitch.max(0.01) as f64))
            .panning(Panning(p.pan.clamp(-1.0, 1.0)));
        if p.looped {
            data = data.loop_region(..);
        }
        if p.fade_in > 0.0 {
            data = data.fade_in_tween(tween(p.fade_in));
        }
        let played = match p.bus {
            None => self.manager.play(data).map_err(|e| e.to_string()),
            Some(bus) => match self.track(bus) {
                Some(track) => track.play(data).map_err(|e| e.to_string()),
                None => return,
            },
        };
        match played {
            Ok(handle) => {
                self.sounds.insert(id, (handle, p.bus));
            }
            Err(e) => log::error!("can't play sound {}: {e}", path.display()),
        }
    }

    fn track(&mut self, bus: BusId) -> Option<&mut TrackHandle> {
        if !self.buses.contains_key(&bus) {
            match self.manager.add_sub_track(TrackBuilder::new()) {
                Ok(t) => {
                    self.buses.insert(bus, t);
                }
                Err(e) => {
                    log::error!("can't create audio bus: {e}");
                    return None;
                }
            }
        }
        self.buses.get_mut(&bus)
    }
}

impl AudioBackend for KiraBackend {
    fn apply(&mut self, commands: Vec<AudioCommand>) {
        self.sounds.retain(|_, (h, _)| h.state() != PlaybackState::Stopped);
        for command in commands {
            match command {
                AudioCommand::Play { id, path, params } => self.play(id, &path, params),
                AudioCommand::Stop { id, fade } => {
                    if let Some((h, _)) = self.sounds.get_mut(&id) {
                        h.stop(tween(fade));
                    }
                }
                AudioCommand::SetVolume { id, volume, fade } => {
                    if let Some((h, _)) = self.sounds.get_mut(&id) {
                        h.set_volume(db(volume), tween(fade));
                    }
                }
                AudioCommand::SetPitch { id, pitch, fade } => {
                    if let Some((h, _)) = self.sounds.get_mut(&id) {
                        h.set_playback_rate(PlaybackRate(pitch.max(0.01) as f64), tween(fade));
                    }
                }
                AudioCommand::SetPan { id, pan } => {
                    if let Some((h, _)) = self.sounds.get_mut(&id) {
                        h.set_panning(Panning(pan.clamp(-1.0, 1.0)), tween(0.0));
                    }
                }
                AudioCommand::StopAll { fade } => {
                    for (h, _) in self.sounds.values_mut() {
                        h.stop(tween(fade));
                    }
                }
                AudioCommand::BusVolume { bus, volume, fade } => {
                    if let Some(t) = self.track(bus) {
                        t.set_volume(db(volume), tween(fade));
                    }
                }
                AudioCommand::StopBus { bus, fade } => {
                    for (h, b) in self.sounds.values_mut() {
                        if *b == Some(bus) {
                            h.stop(tween(fade));
                        }
                    }
                }
                AudioCommand::PauseBus { bus, paused, fade } => {
                    if let Some(t) = self.track(bus) {
                        if paused { t.pause(tween(fade)) } else { t.resume(tween(fade)) }
                    }
                }
                AudioCommand::MasterVolume { volume } => {
                    self.manager.main_track().set_volume(db(volume), tween(0.0));
                }
            }
        }
    }

    fn is_playing(&self, id: SoundId) -> bool {
        self.sounds.get(&id).is_some_and(|(h, _)| !matches!(h.state(), PlaybackState::Stopped | PlaybackState::Stopping))
    }
}
