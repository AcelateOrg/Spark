use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use spark_core::branding::{SPARK_URL, Splash, splash_frame};
use spark_core::{Game, ImageData, VERSION, World, run_frame};
use spark_render::Renderer;
use spark_window::winit;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, CursorIcon, Fullscreen, Icon, Window, WindowId};

/// Flags every Spark game understands.
pub const HELP: &str = "\
Spark Engine flags:
  --headless            run without a window (renders off-screen)
  --frames N            run N frames, then exit (default 1 in headless mode)
  --screenshot PATH     save the final frame as PNG (use with --frames)
  --dump-scene          print all objects (positions, rotations, bounds) at exit
  --dump-scene-json PATH  write the scene as JSON at exit (`-` = stdout)
  --screenshot-every N  also save a PNG every N frames (PATH-00042.png next to --screenshot)
  --input FILE          scripted input for headless runs, one event per line:
                        `FRAME key|mouse_button DOWN|UP NAME`, `FRAME mouse X Y`, `FRAME wheel AMOUNT`, `FRAME text STRING`
  --timeout SECONDS     fail if the run takes longer than this (real time)
  --max-fps N           frame limiter (also when vsync is off; minimized windows sleep anyway)
  --size WxH            output size in pixels, e.g. 1280x720
  --fixed-dt SECONDS    fixed time step (headless default: 1/60)
  --no-vsync            uncapped FPS (benchmarking)
  --mute                no audio (always muted in headless mode)
  --splash              play the Powered by Spark splash first (also: App::splash / game.toml splash = true)
  --no-splash           skip the splash even if the game enables it
  --fullscreen          start in borderless fullscreen
  --windowed            start in a window even if the game asks for fullscreen
  --help                this text
In a window: F11 or Alt+Enter toggles fullscreen, F12 saves a screenshot to ./screenshots/
Env: RUST_LOG=debug for more logs, WGPU_BACKEND=dx12|vulkan|gl to force a GPU API.";

/// Parsed command-line flags.
#[derive(Clone, Debug, Default)]
pub struct RunArgs {
    pub headless: bool,
    pub frames: Option<u64>,
    pub screenshot: Option<PathBuf>,
    pub dump_scene: bool,
    pub dump_scene_json: Option<PathBuf>,
    pub screenshot_every: Option<u64>,
    pub input: Option<PathBuf>,
    pub timeout: Option<f32>,
    pub max_fps: Option<f32>,
    pub size: Option<(u32, u32)>,
    pub fixed_dt: Option<f32>,
    pub no_vsync: bool,
    pub mute: bool,
    /// Force the startup splash on.
    pub splash: bool,
    /// Force the startup splash off.
    pub no_splash: bool,
    pub fullscreen: bool,
    pub windowed: bool,
    pub help: bool,
    /// Arguments Spark does not know (e.g. a script path).
    pub rest: Vec<String>,
}

impl RunArgs {
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Self, String> {
        let mut out = Self::default();
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
            match arg.as_str() {
                "--headless" => out.headless = true,
                "--frames" => {
                    out.frames = Some(value("--frames")?.parse().map_err(|_| "--frames: expected a whole number".to_string())?)
                }
                "--screenshot" => out.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--dump-scene" => out.dump_scene = true,
                "--dump-scene-json" => out.dump_scene_json = Some(PathBuf::from(value("--dump-scene-json")?)),
                "--screenshot-every" => {
                    let n: u64 = value("--screenshot-every")?.parse().map_err(|_| "--screenshot-every: expected a whole number".to_string())?;
                    out.screenshot_every = Some(n.max(1));
                }
                "--input" => out.input = Some(PathBuf::from(value("--input")?)),
                "--timeout" => {
                    out.timeout = Some(value("--timeout")?.parse().map_err(|_| "--timeout: expected seconds".to_string())?)
                }
                "--max-fps" => {
                    out.max_fps = Some(value("--max-fps")?.parse().map_err(|_| "--max-fps: expected a number".to_string())?)
                }
                "--size" => {
                    let v = value("--size")?;
                    let bad = || format!("--size: expected WIDTHxHEIGHT like 1280x720, got '{v}'");
                    let (w, h) = v.split_once(['x', 'X']).ok_or_else(bad)?;
                    out.size = Some((w.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?));
                }
                "--fixed-dt" => {
                    out.fixed_dt = Some(value("--fixed-dt")?.parse().map_err(|_| "--fixed-dt: expected seconds, e.g. 0.016".to_string())?)
                }
                "--no-vsync" => out.no_vsync = true,
                "--mute" => out.mute = true,
                "--splash" => out.splash = true,
                "--no-splash" => out.no_splash = true,
                "--fullscreen" => out.fullscreen = true,
                "--windowed" => out.windowed = true,
                "--help" | "-h" => out.help = true,
                _ => out.rest.push(arg),
            }
        }
        Ok(out)
    }
}

/// Runs a [`Game`] in a window (or headless with `--headless`).
///
/// ```no_run
/// use spark::prelude::*;
/// struct MyGame;
/// impl Game for MyGame {}
/// App::new("My Game").run(MyGame);
/// ```
pub struct App {
    title: String,
    width: u32,
    height: u32,
    vsync: bool,
    max_fps: Option<f32>,
    fullscreen: bool,
    icon: Option<ImageData>,
    fps_in_title: bool,
    splash: bool,
    args: RunArgs,
}

impl App {
    /// Creates the app, sets up logging and reads command-line flags (`--help` lists them).
    pub fn new(title: impl Into<String>) -> Self {
        init_logging();
        let args = match RunArgs::parse(std::env::args().skip(1)) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("error: {e}\n\n{HELP}");
                std::process::exit(2);
            }
        };
        if args.help {
            println!("{HELP}");
            std::process::exit(0);
        }
        install_panic_hook();
        Self {
            title: title.into(),
            width: 1280,
            height: 720,
            vsync: true,
            max_fps: None,
            fullscreen: false,
            icon: None,
            fps_in_title: true,
            splash: false,
            args,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Window size (`--size` overrides it).
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn vsync(mut self, on: bool) -> Self {
        self.vsync = on;
        self
    }

    /// Start in borderless fullscreen (players can toggle with F11 / Alt+Enter; `--windowed` overrides).
    /// Frame limiter (e.g. 60 or 144). Works with vsync off; `None` = no limit (default).
    pub fn max_fps(mut self, fps: Option<f32>) -> Self {
        self.max_fps = fps;
        self
    }

    pub fn fullscreen(mut self, on: bool) -> Self {
        self.fullscreen = on;
        self
    }

    /// Window / taskbar icon (any size; 32..256 px squares look best).
    pub fn icon(mut self, image: ImageData) -> Self {
        self.icon = Some(image);
        self
    }

    /// Show "| 60 fps" in the window title (default on; shipped games usually turn it off).
    pub fn fps_in_title(mut self, on: bool) -> Self {
        self.fps_in_title = on;
        self
    }

    /// Play the "Powered by Spark" splash before `Game::start` (default off; `--splash` / `--no-splash` override).
    pub fn splash(mut self, on: bool) -> Self {
        self.splash = on;
        self
    }

    fn wants_splash(&self) -> bool {
        (self.splash || self.args.splash) && !self.args.no_splash
    }

    pub fn args(&self) -> &RunArgs {
        &self.args
    }

    /// Runs until the window closes. Exits the process with code 1 on fatal errors.
    pub fn run<G: Game>(self, game: G) {
        let result = if self.args.headless { run_headless(self, game) } else { run_windowed(self, game) };
        if let Err(e) = result {
            log::error!("{e}");
            report_fatal(&format!("{e}"));
            std::process::exit(1);
        }
    }
}

/// Writes `spark-crash.log` next to the executable and, on Windows, shows a message box
/// (shipped games have no console, so errors would otherwise be invisible).
fn report_fatal(msg: &str) {
    let text = format!("Spark {VERSION} stopped with an error:\n\n{msg}\n");
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        let _ = std::fs::write(dir.join("spark-crash.log"), &text);
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::GetConsoleWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        // Started from a terminal: the log already shows the error.
        // SAFETY: plain Win32 query without arguments.
        if !unsafe { GetConsoleWindow() }.is_null() {
            return;
        }
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (body, title) = (wide(&text), wide("Spark"));
        // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call.
        unsafe {
            MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
        }
    }
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let msg = match info.payload().downcast_ref::<&str>() {
            Some(s) => s.to_string(),
            None => info.payload().downcast_ref::<String>().cloned().unwrap_or_else(|| "unknown panic".into()),
        };
        let at = info.location().map(|l| format!(" ({}:{})", l.file(), l.line())).unwrap_or_default();
        report_fatal(&format!("internal error: {msg}{at}"));
    }));
}

/// Opens a link in the default browser (errors are only logged).
fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    match result {
        Ok(_) => log::info!("opened {url}"),
        Err(e) => log::warn!("cannot open {url}: {e}"),
    }
}

/// A world with the physics backend installed (and audio unless muted / headless).
fn new_world(args: &RunArgs) -> World {
    let mut world = World::new();
    world.physics.set_backend(Box::new(spark_physics::RapierBackend::new()));
    if !args.headless && !args.mute {
        if let Some(audio) = spark_audio::KiraBackend::new() {
            world.audio.set_backend(Box::new(audio));
        }
    }
    world
}

fn init_logging() {
    let env = env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=error,naga=warn");
    let _ = env_logger::Builder::from_env(env).format_timestamp(None).format_target(false).try_init();
}

fn output_size(app: &App) -> (u32, u32) {
    app.args.size.unwrap_or((app.width, app.height))
}

/// Screenshot + scene dump requested by flags.
fn finish(world: &World, renderer: &mut Renderer, args: &RunArgs, always_render: bool) -> Result<(), String> {
    if args.screenshot.is_some() || always_render {
        let image = renderer.screenshot(world)?;
        if let Some(path) = &args.screenshot {
            image.save_png(path)?;
            log::info!("screenshot saved: {} ({}x{})", path.display(), image.width, image.height);
        }
    }
    if args.dump_scene {
        println!("{}", world.scene.dump());
    }
    if let Some(path) = &args.dump_scene_json {
        let json = world.scene.dump_json();
        if path.as_os_str() == "-" {
            println!("{json}");
        } else {
            std::fs::write(path, json).map_err(|e| format!("--dump-scene-json {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// One scripted input event (`--input`).
#[derive(Clone, Debug, PartialEq)]
enum InputEvent {
    Key(spark_core::Key, bool),
    Button(spark_core::MouseButton, bool),
    Move(f32, f32),
    Wheel(f32),
    Text(String),
}

/// Parses an `--input` script: `FRAME key down space`, `FRAME mouse 100 200`, `#` comments.
fn parse_input_script(src: &str) -> Result<Vec<(u64, InputEvent)>, String> {
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |m: &str| format!("--input line {}: {m}: '{line}'", n + 1);
        let mut parts = line.split_whitespace();
        let frame: u64 = parts.next().and_then(|f| f.parse().ok()).ok_or_else(|| err("expected a frame number first"))?;
        let kind = parts.next().ok_or_else(|| err("missing event"))?;
        let rest: Vec<&str> = parts.collect();
        let down = |s: Option<&&str>| match s.copied() {
            Some("down") | Some("press") => Ok(true),
            Some("up") | Some("release") => Ok(false),
            _ => Err(err("expected down / up")),
        };
        let ev = match kind {
            "key" => {
                let name = rest.get(1).ok_or_else(|| err("missing key name"))?;
                let key = spark_core::Key::from_name(name).ok_or_else(|| err("unknown key"))?;
                InputEvent::Key(key, down(rest.first())?)
            }
            "mouse_button" | "button" => {
                let name = rest.get(1).ok_or_else(|| err("missing button name"))?;
                let b = spark_core::MouseButton::from_name(name).ok_or_else(|| err("unknown mouse button"))?;
                InputEvent::Button(b, down(rest.first())?)
            }
            "mouse" | "move" => {
                let x = rest.first().and_then(|v| v.parse().ok()).ok_or_else(|| err("expected X Y"))?;
                let y = rest.get(1).and_then(|v| v.parse().ok()).ok_or_else(|| err("expected X Y"))?;
                InputEvent::Move(x, y)
            }
            "wheel" => InputEvent::Wheel(rest.first().and_then(|v| v.parse().ok()).ok_or_else(|| err("expected an amount"))?),
            "text" => InputEvent::Text(rest.join(" ")),
            _ => return Err(err("unknown event (key, mouse_button, mouse, wheel, text)")),
        };
        out.push((frame, ev));
    }
    out.sort_by_key(|e| e.0);
    Ok(out)
}

fn apply_input(world: &mut World, ev: &InputEvent) {
    let i = &mut world.input;
    match ev {
        InputEvent::Key(k, d) => i.key_event(*k, *d),
        InputEvent::Button(b, d) => i.mouse_button_event(*b, *d),
        InputEvent::Move(x, y) => i.mouse_moved(spark_core::Vec2::new(*x, *y)),
        InputEvent::Wheel(a) => i.wheel_event(*a),
        InputEvent::Text(t) => i.text_event(t),
    }
}

/// `shot.png` + 42 -> `shot-00042.png`.
fn numbered(path: &std::path::Path, frame: u64) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "frame".into());
    path.with_file_name(format!("{stem}-{frame:05}.png"))
}

fn run_headless<G: Game>(app: App, mut game: G) -> Result<(), String> {
    let (w, h) = output_size(&app);
    let mut renderer = Renderer::new_headless(w, h)?;
    log::info!("Spark {VERSION} (headless {w}x{h}) | GPU: {}", renderer.info());
    let mut world = new_world(&app.args);
    world.canvas.set_output_size(w, h);
    // Headless runs are for checks: the game's splash setting is ignored, only `--splash` plays it.
    let mut splash = (app.args.splash && !app.args.no_splash).then(Splash::new);
    if splash.is_none() {
        game.start(&mut world);
    }
    let dt = app.args.fixed_dt.unwrap_or(1.0 / 60.0);
    let script = match &app.args.input {
        Some(p) => parse_input_script(&std::fs::read_to_string(p).map_err(|e| format!("--input {}: {e}", p.display()))?)?,
        None => Vec::new(),
    };
    let mut next_event = 0;
    let started = Instant::now();
    let shots_base = app.args.screenshot.clone().unwrap_or_else(|| PathBuf::from("frame.png"));
    for _ in 0..app.args.frames.unwrap_or(1) {
        if let Some(limit) = app.args.timeout {
            if started.elapsed().as_secs_f32() > limit {
                return Err(format!("--timeout: the run took longer than {limit} s (frame {})", world.time.frame));
            }
        }
        // Events for the frame about to run (frame numbers start at 1, like time.frame).
        let frame = world.time.frame + 1;
        while next_event < script.len() && script[next_event].0 <= frame {
            apply_input(&mut world, &script[next_event].1);
            next_event += 1;
        }
        if let Some(s) = &mut splash {
            if splash_frame(&mut world, s, dt) {
                continue;
            }
            splash = None;
            game.start(&mut world);
        }
        run_frame(&mut game, &mut world, dt);
        if let Some(n) = app.args.screenshot_every {
            if world.time.frame % n == 0 {
                let path = numbered(&shots_base, world.time.frame);
                renderer.screenshot(&world)?.save_png(&path)?;
                log::info!("screenshot saved: {}", path.display());
            }
        }
        if world.quit_requested() {
            break;
        }
    }
    // Always render once so shader / GPU errors show up even without --screenshot.
    finish(&world, &mut renderer, &app.args, true)?;
    log::info!("headless run done: {} frames, {} objects", world.time.frame, world.scene.len());
    Ok(())
}

fn run_windowed<G: Game>(app: App, game: G) -> Result<(), String> {
    let splash = app.wants_splash().then(Splash::new);
    let event_loop = EventLoop::new().map_err(|e| format!("cannot start the window system: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let mut world = new_world(&app.args);
    world.window.fullscreen = (app.fullscreen || app.args.fullscreen) && !app.args.windowed;
    let mut runner = Runner {
        app,
        game,
        splash,
        world,
        window: None,
        renderer: None,
        applied_fullscreen: false,
        applied_title: None,
        started: false,
        last: now,
        fps_timer: now,
        fps_frames: 0,
        frames: 0,
        screenshot_requested: false,
        minimized: false,
        next_frame: now,
        focused: true,
        cursor_locked: false,
        pointer: false,
        gamepads: spark_window::Gamepads::new(),
        result: Ok(()),
    };
    runner.gamepads.init(&mut runner.world.input);
    event_loop.run_app(&mut runner).map_err(|e| format!("window event loop failed: {e}"))?;
    runner.result
}

struct Runner<G: Game> {
    app: App,
    game: G,
    world: World,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// The startup splash; the game starts when it ends.
    splash: Option<Splash>,
    /// Window state last applied from `world.window`.
    applied_fullscreen: bool,
    applied_title: Option<String>,
    started: bool,
    last: Instant,
    fps_timer: Instant,
    fps_frames: u32,
    frames: u64,
    screenshot_requested: bool,
    /// Window minimized / zero-sized: no rendering, the loop sleeps.
    minimized: bool,
    /// Earliest time of the next frame (frame limiter).
    next_frame: Instant,
    focused: bool,
    /// Cursor grab state currently applied to the window.
    cursor_locked: bool,
    /// Hand cursor shown (over the Spark badge).
    pointer: bool,
    gamepads: spark_window::Gamepads,
    result: Result<(), String>,
}

impl<G: Game> Runner<G> {
    /// Game logic keeps running while minimized (music, timers, network), nothing is drawn.
    fn tick_minimized(&mut self, event_loop: &ActiveEventLoop) {
        if self.splash.is_some() {
            return;
        }
        let now = Instant::now();
        let dt = self.app.args.fixed_dt.unwrap_or((now - self.last).as_secs_f32().min(0.1));
        self.last = now;
        self.gamepads.poll(&mut self.world.input);
        run_frame(&mut self.game, &mut self.world, dt);
        if self.world.quit_requested() {
            event_loop.exit();
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.result = Err(error);
        event_loop.exit();
    }

    /// Hides + captures the cursor while the game asks for it (`input.lock_mouse`) and the window has focus.
    fn apply_cursor_lock(&mut self) {
        let want = self.world.input.lock_mouse && self.focused;
        if want == self.cursor_locked {
            return;
        }
        let Some(window) = &self.window else { return };
        if want {
            let grabbed = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            if let Err(e) = grabbed {
                log::warn!("cannot capture the mouse: {e}");
            }
            window.set_cursor_visible(false);
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
        self.cursor_locked = want;
    }

    /// Hand cursor over the Spark badge; opens the link when it was clicked.
    fn apply_branding(&mut self) {
        if self.world.branding.take_open_request() {
            open_url(SPARK_URL);
        }
        let want = self.world.branding.badge_hovered && !self.cursor_locked;
        if want != self.pointer {
            if let Some(w) = &self.window {
                w.set_cursor(if want { CursorIcon::Pointer } else { CursorIcon::Default });
            }
            self.pointer = want;
        }
    }

    /// Applies fullscreen / title changes requested through `world.window`.
    fn apply_window(&mut self) {
        let Some(window) = &self.window else { return };
        if self.world.window.fullscreen != self.applied_fullscreen {
            self.applied_fullscreen = self.world.window.fullscreen;
            window.set_fullscreen(self.applied_fullscreen.then_some(Fullscreen::Borderless(None)));
        }
        if self.world.window.title != self.applied_title {
            self.applied_title = self.world.window.title.clone();
            window.set_title(self.applied_title.as_deref().unwrap_or(&self.app.title));
        }
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let real_dt = (now - self.last).as_secs_f32();
        self.last = now;
        let dt = self.app.args.fixed_dt.unwrap_or(real_dt.min(0.1));
        self.gamepads.poll(&mut self.world.input);

        if let Some(r) = &self.renderer {
            let (w, h) = r.size();
            self.world.canvas.set_output_size(w, h);
        }
        if let Some(splash) = &mut self.splash {
            if !splash_frame(&mut self.world, splash, dt) {
                // Splash over: build the game (may take a moment), then run from a fresh clock.
                self.splash = None;
                self.game.start(&mut self.world);
                self.last = Instant::now();
                return;
            }
        } else {
            run_frame(&mut self.game, &mut self.world, dt);
        }
        self.apply_cursor_lock();
        self.apply_branding();
        self.apply_window();
        let Some(renderer) = self.renderer.as_mut() else { return };
        let capture = std::mem::take(&mut self.screenshot_requested);
        match renderer.render_and_capture_if(&self.world, capture) {
            Err(e) => {
                self.fail(event_loop, e);
                return;
            }
            Ok(Some(image)) => {
                // Encode + write off the main thread: no hitch when pressing F12.
                let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                let path = PathBuf::from(format!("screenshots/shot-{millis}.png"));
                std::thread::spawn(move || match image.save_png(&path) {
                    Ok(()) => log::info!("screenshot saved: {}", path.display()),
                    Err(e) => log::error!("{e}"),
                });
            }
            Ok(None) => {}
        }
        self.frames += 1;
        self.fps_frames += 1;

        let elapsed = (now - self.fps_timer).as_secs_f32();
        if elapsed >= 1.0 {
            let fps = self.fps_frames as f32 / elapsed;
            self.world.time.fps = fps;
            if let Some(w) = self.window.as_ref().filter(|_| self.app.fps_in_title) {
                let title = self.world.window.title.as_deref().unwrap_or(&self.app.title);
                w.set_title(&format!("{title} | {fps:.0} fps"));
            }
            self.fps_frames = 0;
            self.fps_timer = now;
        }

        let done = self.app.args.frames.is_some_and(|n| self.frames >= n);
        if done || self.world.quit_requested() {
            if let Err(e) = finish(&self.world, renderer, &self.app.args, false) {
                self.result = Err(e);
            }
            event_loop.exit();
        }
    }
}

impl<G: Game> ApplicationHandler for Runner<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (w, h) = output_size(&self.app);
        let icon = self.app.icon.as_ref().and_then(|i| Icon::from_rgba(i.pixels.clone(), i.width, i.height).ok());
        let fullscreen = self.world.window.fullscreen;
        self.applied_fullscreen = fullscreen;
        let attributes = Window::default_attributes()
            .with_title(self.app.title.clone())
            // Logical size: the same window size on 100 % and 200 % (HiDPI / Retina) displays.
            .with_inner_size(winit::dpi::LogicalSize::new(w, h))
            .with_window_icon(icon)
            .with_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => return self.fail(event_loop, format!("cannot create a window: {e}")),
        };
        let size = window.inner_size();
        let vsync = self.app.vsync && !self.app.args.no_vsync;
        match Renderer::new_windowed(window.clone(), size.width.max(1), size.height.max(1), vsync) {
            Ok(r) => {
                log::info!("Spark {VERSION} | GPU: {}", r.info());
                self.renderer = Some(r);
            }
            Err(e) => return self.fail(event_loop, e),
        }
        window.set_ime_allowed(true); // text input via IME (CJK etc.) -> input.text()
        self.window = Some(window.clone());
        self.world.canvas.set_output_size(size.width.max(1), size.height.max(1));
        if !self.started {
            self.started = true;
            if self.splash.is_none() {
                self.game.start(&mut self.world);
            }
        }
        self.last = Instant::now();
        self.fps_timer = self.last;
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            if key.state.is_pressed() && !key.repeat && key.physical_key == PhysicalKey::Code(KeyCode::F12) {
                self.screenshot_requested = true;
            }
            let alt = self.world.input.down(spark_core::Key::LAlt) || self.world.input.down(spark_core::Key::RAlt);
            let toggle = key.physical_key == PhysicalKey::Code(KeyCode::F11)
                || (alt && key.physical_key == PhysicalKey::Code(KeyCode::Enter));
            if key.state.is_pressed() && !key.repeat && toggle {
                self.world.window.fullscreen = !self.world.window.fullscreen;
                self.apply_window();
            }
        }
        if let WindowEvent::Focused(f) = &event {
            self.focused = *f;
        }
        if spark_window::handle_input_event(&mut self.world.input, &event) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.minimized = size.width == 0 || size.height == 0;
                if let Some(r) = self.renderer.as_mut().filter(|_| !self.minimized) {
                    r.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.cursor_locked {
                self.world.input.mouse_motion(spark_core::Vec2::new(delta.0 as f32, delta.1 as f32));
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(w) = &self.window else { return };
        let minimized = self.minimized || w.is_minimized() == Some(true);
        // Minimized: ~20 updates per second, no rendering (saves battery / GPU).
        let max_fps = if minimized { Some(20.0) } else { self.app.args.max_fps.or(self.app.max_fps) };
        let now = Instant::now();
        match max_fps.filter(|f| *f > 0.0) {
            Some(_) if now < self.next_frame => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
            }
            limit => {
                if let Some(fps) = limit {
                    let step = std::time::Duration::from_secs_f32(1.0 / fps);
                    // Catch up at most one frame (no bursts after a hitch).
                    self.next_frame = (self.next_frame + step).max(now);
                }
                event_loop.set_control_flow(ControlFlow::Poll);
                if minimized {
                    self.tick_minimized(event_loop);
                } else {
                    w.request_redraw();
                }
            }
        }
    }
}

#[cfg(test)]
mod input_script_tests {
    use super::*;

    #[test]
    fn parses_events() {
        let ev = parse_input_script("# test\n10 key down space\n5 mouse 100 200\n12 key up space\n20 mouse_button down left\n21 text hi there\n").unwrap();
        assert_eq!(ev[0], (5, InputEvent::Move(100.0, 200.0)));
        assert_eq!(ev[1], (10, InputEvent::Key(spark_core::Key::Space, true)));
        assert_eq!(ev[4], (21, InputEvent::Text("hi there".into())));
        assert!(parse_input_script("x key down space").is_err());
        assert!(parse_input_script("1 key down nokey").is_err());
        assert_eq!(numbered(std::path::Path::new("out/shot.png"), 42), PathBuf::from("out/shot-00042.png"));
    }
}
