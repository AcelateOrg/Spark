//! Message types. Everything is `#[serde(tag = "type")]`: `{"type":"stats","frame":42,...}`.

use serde::{Deserialize, Serialize};

/// Game -> editor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameMessage {
    /// First message after connecting.
    Hello { protocol: u32, engine: String, pid: u32, gpu: String },
    /// Frame numbers, a few times per second.
    Stats(Stats),
    /// The whole scene (see [`SceneSnapshot`]), when the editor subscribed with [`EditorCommand::WatchScene`].
    Scene(SceneSnapshot),
    /// The game logic was paused / resumed (by the editor or the game).
    Paused { paused: bool },
    /// The script host stopped on an error (`None` = the error is gone, e.g. after a hot reload).
    ScriptError { message: Option<String> },
    /// Sent right before the game exits.
    Bye,
}

/// Editor -> game.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditorCommand {
    /// Stop / continue game logic. Rendering continues, so the window stays alive.
    Pause { paused: bool },
    /// While paused: run exactly `frames` frames of logic.
    Step { frames: u32 },
    /// Restart the game (scene cleared, `start()` runs again), like a hot reload.
    Reload,
    /// Send [`GameMessage::Scene`] every `interval_ms` (`None` = stop).
    WatchScene { interval_ms: Option<u32> },
    /// Turn the in-game F1 debug overlay on / off.
    Overlay { on: bool },
    /// Close the game gracefully.
    Quit,
}

/// Per-frame numbers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub frame: u64,
    /// Game time in seconds (scaled, stops while paused).
    pub time: f64,
    pub fps: f32,
    pub frame_ms: f32,
    pub min_frame_ms: f32,
    pub max_frame_ms: f32,
    pub objects: usize,
    pub draw_calls: u32,
    pub culled: u32,
    pub triangles: u64,
    pub internal_size: (u32, u32),
    pub paused: bool,
}

/// The scene as the engine dumps it (`Scene::dump_json`). Unknown fields are ignored, so the engine
/// may add more without breaking older editors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneSnapshot {
    #[serde(default)]
    pub frame: u64,
    pub camera: CameraInfo,
    pub objects: Vec<ObjectInfo>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraInfo {
    pub position: [Option<f32>; 3],
    pub forward: [Option<f32>; 3],
    pub fov: f32,
    pub ortho: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub id: u32,
    #[serde(default)]
    pub name: String,
    pub parent: Option<u32>,
    pub mesh: Option<u32>,
    pub position: [Option<f32>; 3],
    pub world_position: [Option<f32>; 3],
    pub rotation_deg: [Option<f32>; 3],
    pub scale: [Option<f32>; 3],
    pub visible: bool,
    pub bounds: Option<[[Option<f32>; 3]; 2]>,
    pub body: Option<BodyInfo>,
    #[serde(default)]
    pub light: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BodyInfo {
    pub kind: String,
    pub velocity: [Option<f32>; 3],
}

impl SceneSnapshot {
    /// Parses the output of `Scene::dump_json`.
    pub fn from_engine_json(frame: u64, json: &str) -> Result<Self, String> {
        let mut s: Self = serde_json::from_str(json).map_err(|e| format!("bad scene json: {e}"))?;
        s.frame = frame;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let msgs = [
            GameMessage::Hello { protocol: 1, engine: "0.1.0".into(), pid: 7, gpu: "x".into() },
            GameMessage::Stats(Stats { frame: 3, fps: 60.0, ..Default::default() }),
            GameMessage::Paused { paused: true },
            GameMessage::ScriptError { message: Some("main.luau:3: boom".into()) },
            GameMessage::Bye,
        ];
        for m in msgs {
            let line = serde_json::to_string(&m).unwrap();
            assert!(!line.contains('\n'));
            assert_eq!(serde_json::from_str::<GameMessage>(&line).unwrap(), m);
        }
        let c = EditorCommand::WatchScene { interval_ms: Some(250) };
        let line = serde_json::to_string(&c).unwrap();
        assert_eq!(line, r#"{"type":"watch_scene","interval_ms":250}"#);
        assert_eq!(serde_json::from_str::<EditorCommand>(&line).unwrap(), c);
    }

    #[test]
    fn parses_engine_scene_dump() {
        let json = r#"{"camera":{"position":[0.0000,2.0000,5.0000],"forward":[0.0000,0.0000,-1.0000],"fov":60.00,"ortho":null},"objects":[{"id":0,"name":"ground","parent":null,"mesh":1,"position":[0.0,0.0,0.0],"world_position":[0.0,0.0,0.0],"rotation_deg":[0.0,0.0,0.0],"scale":[1.0,1.0,1.0],"visible":true,"bounds":[[-1.0,0.0,-1.0],[1.0,0.0,1.0]],"body":{"kind":"static","velocity":[0.0,0.0,0.0]},"light":false},{"id":1,"name":"","parent":0,"mesh":null,"position":[null,0.0,0.0],"world_position":[0.0,0.0,0.0],"rotation_deg":[0.0,0.0,0.0],"scale":[1.0,1.0,1.0],"visible":false,"bounds":null,"body":null,"light":true}]}"#;
        let s = SceneSnapshot::from_engine_json(9, json).unwrap();
        assert_eq!(s.frame, 9);
        assert_eq!(s.objects.len(), 2);
        assert_eq!(s.objects[1].parent, Some(0));
        assert_eq!(s.objects[1].position[0], None);
        assert_eq!(s.objects[0].body.as_ref().unwrap().kind, "static");
    }
}
