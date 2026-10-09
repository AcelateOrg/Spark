//! Audio commands. The game queues commands; a backend (`spark-audio`, kira) plays them at the end of
//! the frame. Without a backend (headless, tests) everything is silently ignored.

use std::path::PathBuf;

use glam::Vec3;

use crate::scene::{ObjectId, Scene};
use crate::world::World;

/// Where a 3D sound comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundSource {
    Point(Vec3),
    /// Follows the object; stays at its last position if the object is destroyed.
    Object(ObjectId),
}

/// A sound whose volume and pan follow the camera (the listener).
#[derive(Clone, Copy, Debug)]
struct Spatial {
    id: SoundId,
    source: SoundSource,
    last_pos: Vec3,
    volume: f32,
    range: f32,
    fresh: bool,
    sent: (f32, f32),
}

/// Handle to a playing sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundId(pub u64);

/// A mixer bus ("music", "sfx", "ui", ...): a group of sounds with its own volume, stop and pause.
/// Created by name with [`Audio::bus`]. Sounds without a bus go straight to the master output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BusId(pub u32);

/// How to play a sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayParams {
    /// Mixer bus; `None` = master.
    pub bus: Option<BusId>,
    /// Linear volume: 0 = silent, 1 = original, >1 = louder.
    pub volume: f32,
    /// Playback speed: 1 = normal, 2 = one octave up.
    pub pitch: f32,
    /// -1 = left, 0 = center, 1 = right.
    pub pan: f32,
    pub looped: bool,
    /// Fade-in time in seconds.
    pub fade_in: f32,
}

impl Default for PlayParams {
    fn default() -> Self {
        Self { bus: None, volume: 1.0, pitch: 1.0, pan: 0.0, looped: false, fade_in: 0.0 }
    }
}

/// A queued audio operation (`fade` values are in seconds).
#[derive(Clone, Debug, PartialEq)]
pub enum AudioCommand {
    Play { id: SoundId, path: PathBuf, params: PlayParams },
    Stop { id: SoundId, fade: f32 },
    SetVolume { id: SoundId, volume: f32, fade: f32 },
    SetPitch { id: SoundId, pitch: f32, fade: f32 },
    SetPan { id: SoundId, pan: f32 },
    StopAll { fade: f32 },
    MasterVolume { volume: f32 },
    BusVolume { bus: BusId, volume: f32, fade: f32 },
    /// Stops every sound on the bus.
    StopBus { bus: BusId, fade: f32 },
    /// Pauses (`true`) or resumes every sound on the bus.
    PauseBus { bus: BusId, paused: bool, fade: f32 },
    /// Forget every decoded sound (game reload): frees memory, changed files are read again.
    ClearCache,
}

#[derive(Clone, Debug)]
struct BusState {
    name: String,
    volume: f32,
    paused: bool,
}

/// Something that can play sounds (see `spark-audio`).
pub trait AudioBackend {
    /// Executes the queued commands of this frame (in order).
    fn apply(&mut self, commands: Vec<AudioCommand>);
    /// Is this sound still playing?
    fn is_playing(&self, id: SoundId) -> bool;
}

/// Audio queue + backend.
#[derive(Default)]
pub struct Audio {
    commands: Vec<AudioCommand>,
    next_id: u64,
    master_volume: f32,
    master_set: bool,
    spatial: Vec<Spatial>,
    buses: Vec<BusState>,
    backend: Option<Box<dyn AudioBackend>>,
}

impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio")
            .field("queued", &self.commands.len())
            .field("master_volume", &self.master_volume())
            .field("backend", &self.backend.is_some())
            .finish()
    }
}

impl Audio {
    pub fn set_backend(&mut self, backend: Box<dyn AudioBackend>) {
        self.backend = Some(backend);
    }

    pub fn has_backend(&self) -> bool {
        self.backend.is_some()
    }

    /// Queues a sound. `path` should already be resolved (see `Assets::resolve`).
    pub fn play(&mut self, path: impl Into<PathBuf>, params: PlayParams) -> SoundId {
        self.next_id += 1;
        let id = SoundId(self.next_id);
        self.commands.push(AudioCommand::Play { id, path: path.into(), params });
        id
    }

    /// Queues a 3D sound: louder near the camera, silent beyond `range` meters, panned left/right.
    pub fn play_at(&mut self, path: impl Into<PathBuf>, params: PlayParams, source: SoundSource, range: f32) -> SoundId {
        let id = self.play(path, params);
        let last_pos = match source {
            SoundSource::Point(p) => p,
            SoundSource::Object(_) => Vec3::ZERO,
        };
        self.spatial.push(Spatial {
            id,
            source,
            last_pos,
            volume: params.volume,
            range: range.max(0.01),
            fresh: true,
            sent: (-1.0, 0.0),
        });
        id
    }

    /// Moves a 3D sound (no effect on normal sounds).
    pub fn set_source(&mut self, id: SoundId, source: SoundSource) {
        if let Some(s) = self.spatial.iter_mut().find(|s| s.id == id) {
            s.source = source;
        }
    }

    pub fn is_spatial(&self, id: SoundId) -> bool {
        self.spatial.iter().any(|s| s.id == id)
    }

    /// Recomputes volume / pan of 3D sounds from the camera. Called by `run_frame` before `flush`.
    pub fn update_spatial(&mut self, scene: &Scene) {
        if self.spatial.is_empty() {
            return;
        }
        let cam = scene.camera.position;
        let right = scene.camera.right();
        let mut spatial = std::mem::take(&mut self.spatial);
        spatial.retain(|s| s.fresh || self.is_playing(s.id));
        for s in &mut spatial {
            if let SoundSource::Object(o) = s.source {
                if scene.contains(o) {
                    s.last_pos = scene.world_position(o);
                }
            } else if let SoundSource::Point(p) = s.source {
                s.last_pos = p;
            }
            let to = s.last_pos - cam;
            let d = to.length();
            let near = (1.0 - d / s.range).clamp(0.0, 1.0);
            let gain = near * near * s.volume;
            // Close sounds are centered; far ones are panned harder.
            let pan = if d > 0.001 { to.normalize().dot(right) * 0.85 * (d / 2.0).min(1.0) } else { 0.0 };
            if s.fresh {
                s.fresh = false;
                for c in self.commands.iter_mut() {
                    if let AudioCommand::Play { id, params, .. } = c {
                        if *id == s.id {
                            params.volume = gain;
                            params.pan = pan;
                        }
                    }
                }
                s.sent = (gain, pan);
                continue;
            }
            if (gain - s.sent.0).abs() > 0.002 {
                self.commands.push(AudioCommand::SetVolume { id: s.id, volume: gain, fade: 0.05 });
            }
            if (pan - s.sent.1).abs() > 0.01 {
                self.commands.push(AudioCommand::SetPan { id: s.id, pan });
            }
            s.sent = (gain, pan);
        }
        self.spatial = spatial;
    }

    /// Bus by name, created on first use (volume 1, playing).
    pub fn bus(&mut self, name: &str) -> BusId {
        if let Some(i) = self.buses.iter().position(|b| b.name == name) {
            return BusId(i as u32);
        }
        self.buses.push(BusState { name: name.to_string(), volume: 1.0, paused: false });
        BusId(self.buses.len() as u32 - 1)
    }

    /// Names of all buses created so far.
    pub fn bus_names(&self) -> Vec<&str> {
        self.buses.iter().map(|b| b.name.as_str()).collect()
    }

    pub fn bus_volume(&self, bus: BusId) -> f32 {
        self.buses.get(bus.0 as usize).map_or(1.0, |b| b.volume)
    }

    pub fn set_bus_volume(&mut self, bus: BusId, volume: f32, fade: f32) {
        if let Some(b) = self.buses.get_mut(bus.0 as usize) {
            b.volume = volume.max(0.0);
            self.commands.push(AudioCommand::BusVolume { bus, volume: b.volume, fade });
        }
    }

    pub fn bus_paused(&self, bus: BusId) -> bool {
        self.buses.get(bus.0 as usize).is_some_and(|b| b.paused)
    }

    pub fn pause_bus(&mut self, bus: BusId, paused: bool, fade: f32) {
        if let Some(b) = self.buses.get_mut(bus.0 as usize) {
            b.paused = paused;
            self.commands.push(AudioCommand::PauseBus { bus, paused, fade });
        }
    }

    pub fn stop_bus(&mut self, bus: BusId, fade: f32) {
        self.commands.push(AudioCommand::StopBus { bus, fade });
    }

    pub fn stop(&mut self, id: SoundId, fade: f32) {
        self.commands.push(AudioCommand::Stop { id, fade });
    }

    pub fn set_volume(&mut self, id: SoundId, volume: f32, fade: f32) {
        if let Some(s) = self.spatial.iter_mut().find(|s| s.id == id) {
            s.volume = volume;
            return;
        }
        self.commands.push(AudioCommand::SetVolume { id, volume, fade });
    }

    pub fn set_pitch(&mut self, id: SoundId, pitch: f32, fade: f32) {
        self.commands.push(AudioCommand::SetPitch { id, pitch, fade });
    }

    pub fn set_pan(&mut self, id: SoundId, pan: f32) {
        self.commands.push(AudioCommand::SetPan { id, pan });
    }

    /// Drops the backend's decoded sounds (called on game reload).
    pub fn clear_cache(&mut self) {
        self.commands.push(AudioCommand::ClearCache);
    }

    pub fn stop_all(&mut self, fade: f32) {
        self.spatial.clear();
        self.commands.push(AudioCommand::StopAll { fade });
    }

    pub fn master_volume(&self) -> f32 {
        if self.master_set { self.master_volume } else { 1.0 }
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.master_volume = volume.max(0.0);
        self.master_set = true;
        self.commands.push(AudioCommand::MasterVolume { volume: self.master_volume });
    }

    /// `true` while the sound plays (also for sounds queued this frame). Always `false` without a backend
    /// once the frame that queued it has ended.
    pub fn is_playing(&self, id: SoundId) -> bool {
        let queued = self.commands.iter().rev().find_map(|c| match c {
            AudioCommand::Play { id: i, .. } if *i == id => Some(true),
            AudioCommand::Stop { id: i, .. } if *i == id => Some(false),
            AudioCommand::StopAll { .. } => Some(false),
            _ => None,
        });
        match queued {
            Some(v) => v && self.backend.is_some(),
            None => self.backend.as_ref().is_some_and(|b| b.is_playing(id)),
        }
    }

    /// Commands waiting for the end of the frame.
    pub fn pending(&self) -> &[AudioCommand] {
        &self.commands
    }

    /// Sends queued commands to the backend (or drops them). Called by `run_frame`.
    pub fn flush(&mut self) {
        let commands = std::mem::take(&mut self.commands);
        if let Some(b) = self.backend.as_mut() {
            if !commands.is_empty() {
                b.apply(commands);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buses_are_named_and_keep_state() {
        let mut a = Audio::default();
        let music = a.bus("music");
        let sfx = a.bus("sfx");
        assert_eq!(a.bus("music"), music);
        assert_ne!(music, sfx);
        a.set_bus_volume(music, 0.25, 1.0);
        a.pause_bus(sfx, true, 0.0);
        assert_eq!(a.bus_volume(music), 0.25);
        assert_eq!(a.bus_volume(sfx), 1.0);
        assert!(a.bus_paused(sfx) && !a.bus_paused(music));
        a.play("x.wav", PlayParams { bus: Some(music), ..Default::default() });
        assert!(matches!(a.pending().last(), Some(AudioCommand::Play { params, .. }) if params.bus == Some(music)));
        assert_eq!(a.bus_names(), vec!["music", "sfx"]);
    }
}

/// Flushes the audio queue of `world`.
pub fn flush(world: &mut World) {
    world.audio.update_spatial(&world.scene);
    world.audio.flush();
}
