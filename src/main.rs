//! saturn-art — a live, realistic Saturn in colored ASCII art.
//!
//! Usage:
//!   saturn-art [--fps N] [--speed X] [--tilt DEG] [--azimuth DEG] [--dist R]
//!   saturn-art --snapshot out.ppm [--cols N] [--rows N] [--time SECS]
//!   saturn-art --snapshot - --ascii [--cols N] [--rows N]   (text)
//!   saturn-art --snapshot - [--cols N] [--rows N] > out.ppm
//!
//! Keys while running:
//!
//! * `q` / `Esc` quit
//! * `space` pause
//! * `+` / `-` speed up / slow down
//! * `c` cycle the glyph ramp
//! * `s` toggle stars
//! * `r` reset

use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, event, execute, terminal};

mod color;
mod scene;

use scene::{CHARSETS, Params, V3};

const SPIN_RATE: f32 = 0.30; // radians / second, ~21 s per rotation
// Default view: rings tipped up to the right, with the sun nearly behind the
// camera so the globe stays bright and peachy.  The live camera wanders.
const BASE_TILT_DEG: f32 = 15.0;
const BASE_AZIMUTH_DEG: f32 = 10.0;
const BASE_ROLL_DEG: f32 = 12.0;
const CAMERA_DISTANCE: f32 = 6.5;
const SUN: V3 = V3::new(-0.18, 0.03, -0.983);

// ---------------------------------------------------------------------------
// options
// ---------------------------------------------------------------------------

struct Options {
    fps: u32,
    speed: f32,
    tilt_deg: f32,
    azimuth_deg: f32,
    roll_deg: f32,
    distance: f32,
    zoom: f32,
    sun: V3,
    stars: bool,
    snapshot: Option<String>,
    cols: usize,
    rows: usize,
    time: f32,
    ascii: bool,
    charset: usize,
    screensaver: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            fps: 30,
            speed: 1.0,
            tilt_deg: BASE_TILT_DEG,
            azimuth_deg: BASE_AZIMUTH_DEG,
            roll_deg: BASE_ROLL_DEG,
            distance: CAMERA_DISTANCE,
            zoom: 1.0,
            sun: SUN,
            stars: true,
            snapshot: None,
            cols: 160,
            rows: 50,
            time: 0.0,
            ascii: false,
            charset: 0,
            screensaver: false,
        }
    }
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options::default();
    let mut args = std::env::args().skip(1);

    while let Some(a) = args.next() {
        let (key, inline) = match a.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (a.clone(), None),
        };
        let mut next = |name: &str| -> Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            args.next().ok_or_else(|| format!("{name} needs a value"))
        };

        match key.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("saturn-art {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--fps" => {
                o.fps = next("--fps")?.parse().map_err(|_| "bad --fps")?;
                o.fps = o.fps.clamp(5, 120);
            }
            "--speed" => {
                o.speed = next("--speed")?.parse().map_err(|_| "bad --speed")?;
                o.speed = o.speed.clamp(0.05, 8.0);
            }
            "--tilt" => {
                o.tilt_deg = next("--tilt")?.parse().map_err(|_| "bad --tilt")?;
                o.tilt_deg = o.tilt_deg.clamp(-85.0, 85.0);
            }
            "--azimuth" | "--angle" => {
                o.azimuth_deg = next("--azimuth")?.parse().map_err(|_| "bad --azimuth")?;
                o.azimuth_deg = o.azimuth_deg.clamp(-360.0, 360.0);
            }
            "--view" => {
                match next("--view")?.as_str() {
                    // straight-on postcard
                    "classic" => {
                        o.tilt_deg = 24.0;
                        o.azimuth_deg = 0.0;
                        o.roll_deg = 0.0;
                        o.distance = 14.0;
                        o.sun = V3::new(-0.45, 0.42, -0.79);
                    }
                    // rings sweeping diagonally across the frame
                    "dramatic" => {
                        o.tilt_deg = 27.0;
                        o.azimuth_deg = 26.0;
                        o.roll_deg = -14.0;
                        o.distance = 6.0;
                        o.sun = V3::new(-0.48, 0.44, -0.76);
                    }
                    // low, close and deep
                    "closeup" => {
                        o.tilt_deg = 16.0;
                        o.azimuth_deg = 18.0;
                        o.roll_deg = -8.0;
                        o.distance = 3.9;
                        o.sun = V3::new(-0.50, 0.40, -0.77);
                    }
                    // almost edge-on rings slicing the globe (default)
                    "edge" => {
                        o.tilt_deg = BASE_TILT_DEG;
                        o.azimuth_deg = BASE_AZIMUTH_DEG;
                        o.roll_deg = BASE_ROLL_DEG;
                        o.distance = CAMERA_DISTANCE;
                        o.sun = SUN;
                    }
                    other => return Err(format!("unknown view: {other}")),
                }
            }
            "--roll" => {
                o.roll_deg = next("--roll")?.parse().map_err(|_| "bad --roll")?;
                o.roll_deg = o.roll_deg.clamp(-180.0, 180.0);
            }
            "--dist" => {
                o.distance = next("--dist")?.parse().map_err(|_| "bad --dist")?;
                o.distance = o.distance.clamp(3.6, 60.0);
            }
            "--zoom" => {
                o.zoom = next("--zoom")?.parse().map_err(|_| "bad --zoom")?;
                o.zoom = o.zoom.clamp(0.5, 3.0);
            }
            "--charset" => {
                let v = next("--charset")?;
                o.charset = CHARSETS
                    .iter()
                    .position(|c| c.name == v)
                    .or_else(|| v.parse::<usize>().ok())
                    .filter(|i| *i < CHARSETS.len())
                    .ok_or_else(|| format!("unknown charset: {v}"))?;
            }
            "--screensaver" => o.screensaver = true,
            "--no-stars" => o.stars = false,
            "--snapshot" => o.snapshot = Some(next("--snapshot")?),
            "--cols" => {
                o.cols = next("--cols")?.parse().map_err(|_| "bad --cols")?;
                o.cols = o.cols.clamp(8, 600);
            }
            "--rows" => {
                o.rows = next("--rows")?.parse().map_err(|_| "bad --rows")?;
                o.rows = o.rows.clamp(6, 400);
            }
            "--time" => {
                o.time = next("--time")?.parse().map_err(|_| "bad --time")?;
            }
            "--ascii" => o.ascii = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(o)
}

fn print_help() {
    println!(
        "\
A Rust CLI that renders a live, ray-traced Saturn

Usage
  saturn-art                              run it
  saturn-art --snapshot out.ppm           render one frame to a PPM image
  saturn-art --snapshot - --ascii         print a plain-text frame
  saturn-art --help                       this text

Options
  --view NAME      edge | classic | dramatic | closeup
  --charset NAME   glyph ramp (see Charsets)           (classic)
  --fps N          animation frame rate, 5..120       (30)
  --speed X        time multiplier                    (1.0)
  --tilt DEG       camera elevation, -85..85 deg      (15)
  --azimuth DEG    camera angle around the planet     (10)
  --roll DEG       camera roll around its axis        (12)
  --dist R         camera distance in planet radii    (6.5)
  --zoom X         framing zoom, 0.5..3.0             (1.0)
  --screensaver    any key or click quits (used by the Omarchy screensaver)
  --no-stars       no star field
  --snapshot PATH  render a single frame and exit
  --cols N         frame width  for --snapshot        (160)
  --rows N         frame height for --snapshot        (50)
  --time SECS      scene time for --snapshot          (0)
  --ascii          with --snapshot -, print plain text

Screensaver (Omarchy)
  saturn-art screensaver on       install it as the idle screensaver
  saturn-art screensaver off      restore the stock Omarchy screensaver
  saturn-art screensaver status   show what is active
  saturn-art screensaver start    show it now (any input dismisses)

Views
  edge       rings tipped up to the right, bright globe  (default)
  classic    straight-on postcard, rings open, no roll
  dramatic   angled and rolled, near rings larger
  closeup    low camera, strong perspective, rings wrap the globe

Charsets
  classic   detailed   soft   blocks   dense   dots   minimal   mixed

Keys
  q / Esc   quit            space   pause
  + / -     speed           c       cycle charset
  s         stars           r       reset"
    );
}

// ---------------------------------------------------------------------------
// terminal plumbing
// ---------------------------------------------------------------------------

struct TermGuard {
    mouse: bool,
}

impl TermGuard {
    fn enter(screensaver: bool) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        if screensaver {
            let _ = execute!(io::stdout(), event::EnableMouseCapture);
        }
        Ok(Self { mouse: screensaver })
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        if self.mouse {
            let _ = execute!(io::stdout(), event::DisableMouseCapture);
        }
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), event::DisableMouseCapture);
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        default(info);
    }));
}

// ---------------------------------------------------------------------------
// live mode
// ---------------------------------------------------------------------------

/// Tiny deterministic-per-run PRNG (xorshift64*).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }
    fn unit(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }
}

/// A camera that wanders: every few seconds it picks a new pose nearby and
/// eases toward it, with the ranges clamped to keep Saturn readable.  The
/// seed comes from the clock, so every run takes a different path.
struct Cam {
    tilt: f32,
    az: f32,
    roll: f32,
    t_tilt: f32,
    t_az: f32,
    t_roll: f32,
    next: f32,
    base_az: f32,
}

impl Cam {
    fn new(opts: &Options, rng: &mut Rng) -> Self {
        // Begin somewhere random inside the allowed range, so the very first
        // frame of every run is different.  The roll may be either sign: the
        // rings can tip up-left as well as up-right.
        let mut c = Self {
            tilt: rng.range(8.0, 26.0),
            az: opts.azimuth_deg + rng.range(-36.0, 36.0),
            roll: rng.range(-30.0, 30.0),
            t_tilt: 0.0,
            t_az: 0.0,
            t_roll: 0.0,
            next: 0.0,
            base_az: opts.azimuth_deg,
        };
        c.t_tilt = c.tilt;
        c.t_az = c.az;
        c.t_roll = c.roll;
        c.repick(rng, 0.0);
        c
    }

    fn repick(&mut self, rng: &mut Rng, now: f32) {
        // Keep the phase around the sun and the roll moderate, whichever
        // shot is chosen.
        self.t_az = self.base_az + rng.range(-36.0, 36.0);
        self.t_roll = (self.roll + rng.range(-12.0, 12.0)).clamp(-32.0, 32.0);

        self.t_tilt = (self.tilt + rng.range(-6.0, 6.0)).clamp(8.0, 26.0);
        self.next = now + rng.range(5.0, 10.0);
    }

    fn advance(&mut self, rng: &mut Rng, t: f32, dt: f32) {
        if t >= self.next {
            self.repick(rng, t);
        }
        let k = 1.0 - (-dt / 4.0).exp();
        self.tilt += (self.t_tilt - self.tilt) * k;
        self.az += (self.t_az - self.az) * k;
        self.roll += (self.t_roll - self.roll) * k;
    }
}

fn run(opts: Options) -> io::Result<()> {
    install_panic_hook();
    let _guard = TermGuard::enter(opts.screensaver)?;

    let mut out = BufWriter::with_capacity(1 << 19, io::stdout().lock());

    let frame_target = Duration::from_secs_f32(1.0 / opts.fps as f32);
    let mut last_start = Instant::now();

    let mut t: f32 = 0.0;
    let mut speed = opts.speed;
    let mut paused = false;
    let mut stars = opts.stars;
    let mut charset = opts.charset;
    let mut status: Option<(String, f32)> = None;
    let mut dims = (0usize, 0usize);
    let mut prev: Option<Vec<scene::Cell>> = None;
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5a7);
    let mut rng = Rng::new(seed);
    let mut cam = Cam::new(&opts, &mut rng);

    'main: loop {
        let frame_start = Instant::now();
        let dt = (frame_start - last_start).as_secs_f32().min(0.1);
        last_start = frame_start;

        // --- input -------------------------------------------------------
        while event::poll(Duration::ZERO)? {
            // In screensaver mode any real input (key, click, scroll, or
            // pointer movement) dismisses it.  Resize and focus events are
            // ignored: a freshly mapped window emits them immediately.
            if opts.screensaver {
                match event::read()? {
                    Event::Key(_) | Event::Mouse(_) => break 'main,
                    _ => continue,
                }
            }
            let Event::Key(k) = event::read()? else {
                continue;
            };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break 'main,
                KeyCode::Char('c') if ctrl => break 'main,
                KeyCode::Char(' ') => {
                    paused = !paused;
                    status = Some((if paused { "paused" } else { "resumed" }.into(), 2.5));
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    speed = (speed * 1.25).min(8.0);
                    status = Some((format!("speed {speed:.2}x"), 2.5));
                }
                KeyCode::Char('-') | KeyCode::Char('_') => {
                    speed = (speed / 1.25).max(0.2);
                    status = Some((format!("speed {speed:.2}x"), 2.5));
                }
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    charset = (charset + 1) % CHARSETS.len();
                    status = Some((format!("charset: {}", CHARSETS[charset].name), 2.5));
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    stars = !stars;
                    status = Some((format!("stars: {}", if stars { "on" } else { "off" }), 2.5));
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    t = 0.0;
                    speed = opts.speed;
                    paused = false;
                    status = Some(("reset".into(), 2.5));
                }
                _ => {}
            }
        }

        if !paused {
            let sdt = dt * speed;
            t += sdt;
            cam.advance(&mut rng, t, sdt);
        }

        let (cols, rows) = terminal::size()?;
        // The renderer clamps to a minimum size; keep the indexing in sync.
        let cols = (cols as usize).max(4);
        let rows = (rows as usize).max(3);

        let p = Params {
            tilt: cam.tilt.to_radians(),
            azimuth: cam.az.to_radians(),
            roll: cam.roll.to_radians(),
            distance: opts.distance,
            zoom: opts.zoom,
            spin: t * SPIN_RATE,
            sun: opts.sun,
            time: t,
            stars,
            ramp: CHARSETS[charset].ramp,
        };
        // Reuse the previous frame's glyphs as hysteresis, but only when the
        // terminal geometry is unchanged.
        let prev_ref = prev.as_deref().filter(|_| dims == (cols, rows));
        let cells = scene::render(cols, rows, &p, prev_ref);
        prev = Some(cells.clone());

        let mut s = String::with_capacity(cols * rows * 8 + 64);
        let mut last_rgb = (0u8, 0u8, 0u8);
        if dims != (cols, rows) {
            // New geometry: wipe whatever the previous layout left behind,
            // and make sure the cursor stays hidden after a window change.
            s.push_str("\x1b[2J");
            dims = (cols, rows);
        }
        s.push_str("\x1b[?25l\x1b[1;1H");
        for j in 0..rows {
            if j > 0 {
                let _ = write!(s, "\x1b[{};1H", j + 1);
            }
            for i in 0..cols {
                let c = cells[j * cols + i];
                if c.ch == b' ' {
                    s.push(' ');
                    continue;
                }
                // Reuse the current color when the next cell is within a
                // couple of levels: imperceptible, and it saves a lot of
                // escape bytes on smooth gradients.
                let close = c.r.abs_diff(last_rgb.0) <= 2
                    && c.g.abs_diff(last_rgb.1) <= 2
                    && c.b.abs_diff(last_rgb.2) <= 2;
                if !close {
                    let _ = write!(s, "\x1b[38;2;{};{};{}m", c.r, c.g, c.b);
                    last_rgb = (c.r, c.g, c.b);
                }
                s.push(c.ch as char);
            }
        }

        if let Some((_, left)) = &mut status {
            *left -= dt;
            if *left <= 0.0 {
                status = None;
            }
        }

        // No permanent footer: only a brief message when a key changes
        // something, drawn over the last row of sky.
        if let Some((msg, _)) = &status {
            let _ = write!(s, "\x1b[{};1H\x1b[38;2;120;120;132m", rows);
            let mut text = String::from(msg);
            if paused {
                text.push_str("   paused");
            }
            let mut used = 0usize;
            for ch in text.chars() {
                if used >= cols {
                    break;
                }
                s.push(ch);
                used += 1;
            }
            while used < cols {
                s.push(' ');
                used += 1;
            }
        }

        s.push_str("\x1b[0m\x1b[J");
        out.write_all(s.as_bytes())?;
        out.flush()?;

        let spent = frame_start.elapsed();
        if spent < frame_target {
            std::thread::sleep(frame_target - spent);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// snapshot mode (for previews / screenshots)
// ---------------------------------------------------------------------------

fn snapshot(opts: &Options) -> io::Result<()> {
    let p = Params {
        tilt: opts.tilt_deg.to_radians(),
        azimuth: opts.azimuth_deg.to_radians(),
        roll: opts.roll_deg.to_radians(),
        distance: opts.distance,
        zoom: opts.zoom,
        spin: opts.time * SPIN_RATE,
        sun: opts.sun,
        time: opts.time,
        stars: opts.stars,
        ramp: CHARSETS[opts.charset].ramp,
    };
    let cells = scene::render(opts.cols, opts.rows, &p, None);

    if opts.ascii {
        let mut s = String::new();
        for j in 0..opts.rows {
            for i in 0..opts.cols {
                s.push(cells[j * opts.cols + i].ch as char);
            }
            s.push('\n');
        }
        print!("{s}");
        return Ok(());
    }

    let path = opts.snapshot.as_deref().unwrap_or("saturn.ppm");

    // Each character cell becomes a 3x6 pixel block, matching the 1:2 cell
    // aspect ratio so the preview has the same shape as the terminal output.
    let (sx, sy) = (3usize, 6usize);
    let w = opts.cols * sx;
    let h = opts.rows * sy;
    let mut buf = Vec::with_capacity(w * h * 3);
    for j in 0..opts.rows {
        for _sy in 0..sy {
            for i in 0..opts.cols {
                let c = cells[j * opts.cols + i];
                for _sx in 0..sx {
                    buf.push(c.r);
                    buf.push(c.g);
                    buf.push(c.b);
                }
            }
        }
    }
    let header = format!("P6\n{} {}\n255\n", w, h);
    if path == "-" {
        let stdout = io::stdout();
        let mut f = BufWriter::new(stdout.lock());
        f.write_all(header.as_bytes())?;
        f.write_all(&buf)?;
        f.flush()?;
    } else {
        let mut f = BufWriter::new(std::fs::File::create(path)?);
        f.write_all(header.as_bytes())?;
        f.write_all(&buf)?;
        f.flush()?;
        eprintln!("wrote {path} ({w}x{h})");
    }
    Ok(())
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Omarchy screensaver management
// ---------------------------------------------------------------------------

const SCREENSAVER_LAUNCHER: &str = include_str!("../omarchy/omarchy-launch-screensaver");
const PATH_BEGIN: &str = "# >>> saturn-art screensaver >>>";
const PATH_END: &str = "# <<< saturn-art screensaver <<<";

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/share"))
}

fn shim_path() -> PathBuf {
    data_home().join("saturn-art/bin/omarchy-launch-screensaver")
}

fn on_path(command: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {command}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn login_which(command: &str) -> Option<String> {
    let out = Command::new("bash")
        .arg("-lc")
        .arg(format!("command -v {command}"))
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !path.is_empty() {
        Some(path)
    } else {
        None
    }
}

fn strip_block(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut skip = false;
    for line in text.lines() {
        if line == PATH_BEGIN {
            skip = true;
            continue;
        }
        if line == PATH_END {
            skip = false;
            continue;
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn write_block(path: &Path, shim_dir: &Path) -> io::Result<()> {
    let old = fs::read_to_string(path).unwrap_or_default();
    let mut text = strip_block(&old);
    while text.ends_with("\n\n") {
        text.pop();
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(PATH_BEGIN);
    text.push('\n');
    let _ = write!(text, "export PATH=\"{}:$PATH\"", shim_dir.display());
    text.push('\n');
    text.push_str(PATH_END);
    text.push('\n');
    fs::write(path, text)
}

fn clean_block(path: &Path) -> io::Result<()> {
    if let Ok(old) = fs::read_to_string(path) {
        let cleaned = strip_block(&old);
        if cleaned != old {
            fs::write(path, cleaned)?;
        }
    }
    Ok(())
}

fn idle_seconds(key: &str) -> Option<u32> {
    let path = home_dir().join(".config/omarchy/shell.json");
    let text = fs::read_to_string(path).ok()?;
    let at = text.find(&format!("\"{key}\""))?;
    let rest = &text[at + key.len() + 2..];
    let rest = &rest[rest.find(':')? + 1..];
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn screensaver_install() -> io::Result<()> {
    if !Path::new("/usr/share/omarchy").exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Omarchy was not found on this system",
        ));
    }
    if !on_path("saturn-art") {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "saturn-art is not on PATH; run `make install` first",
        ));
    }

    let shim = shim_path();
    let dir = shim.parent().unwrap_or(Path::new(".")).to_path_buf();
    fs::create_dir_all(&dir)?;
    fs::write(&shim, SCREENSAVER_LAUNCHER)?;
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755))?;

    // Login shells are what Omarchy's idle service and menu invoke the
    // launcher through, so the PATH block belongs in ~/.bash_profile.
    write_block(&home_dir().join(".bash_profile"), &dir)?;
    // Older versions used ~/.bashrc; that lands before the mise env and is
    // overwritten, so drop it.
    clean_block(&home_dir().join(".bashrc"))?;

    println!("saturn-art is now the Omarchy screensaver.");
    println!("  show it now:  saturn-art screensaver start");
    println!("  turn it off:  saturn-art screensaver off");
    println!(
        "  idle delay:   {}s (idle.screensaver in ~/.config/omarchy/shell.json)",
        idle_seconds("screensaver").unwrap_or(150)
    );
    match login_which("omarchy-launch-screensaver").as_deref() {
        Some(p) if Path::new(p) == shim => {}
        _ => println!(
            "warning: a login shell does not resolve the shim first; check the PATH block in ~/.bash_profile"
        ),
    }
    Ok(())
}

fn screensaver_remove() -> io::Result<()> {
    let shim = shim_path();
    let _ = fs::remove_file(&shim);
    if let Some(dir) = shim.parent() {
        let _ = fs::remove_dir(dir);
    }
    let _ = fs::remove_dir(data_home().join("saturn-art"));

    clean_block(&home_dir().join(".bash_profile"))?;
    clean_block(&home_dir().join(".bashrc"))?;

    println!("The Omarchy screensaver is back to the stock one.");
    Ok(())
}

fn screensaver_status() {
    let shim = shim_path();
    let installed = shim.exists();
    let resolved = login_which("omarchy-launch-screensaver");
    let resolved_ours = resolved
        .as_deref()
        .map(|p| Path::new(p) == shim)
        .unwrap_or(false);
    let idle_off = Command::new("omarchy-toggle-enabled")
        .arg("screensaver-off")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    println!("Omarchy screensaver");
    println!(
        "  shim:             {}",
        if installed {
            shim.display().to_string()
        } else {
            "not installed".into()
        }
    );
    println!(
        "  login shell:      {}",
        match resolved.as_deref() {
            Some(p) if resolved_ours => format!("{p} (saturn-art)"),
            Some(p) => format!("{p} (stock)"),
            None => "omarchy-launch-screensaver not found".into(),
        }
    );
    println!(
        "  idle screensaver: {}",
        if idle_off {
            "disabled by `omarchy toggle screensaver`"
        } else {
            "enabled"
        }
    );
    println!(
        "  idle delay:       {}s screensaver, {}s lock",
        idle_seconds("screensaver").unwrap_or(150),
        idle_seconds("lock").unwrap_or(300)
    );
    if !installed {
        println!("  next:             saturn-art screensaver on");
    }
}

fn screensaver_start() -> io::Result<()> {
    if !shim_path().exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the screensaver is not installed; run `saturn-art screensaver on`",
        ));
    }
    let status = Command::new("bash")
        .arg("-lc")
        .arg("omarchy-launch-screensaver force")
        .status()?;
    if !status.success() {
        return Err(io::Error::other(
            "the launcher failed to start the screensaver",
        ));
    }
    Ok(())
}

fn screensaver_help() {
    println!(
        "\
Use saturn-art as the Omarchy screensaver

Usage
  saturn-art screensaver on       install it as the idle screensaver
  saturn-art screensaver off      restore the stock Omarchy screensaver
  saturn-art screensaver status   show what is active
  saturn-art screensaver start    show it now (any input dismisses)

The shim is installed at ~/.local/share/saturn-art/bin and a marked PATH
block goes into ~/.bash_profile, because Omarchy runs the screensaver
launcher through a login shell."
    );
}

fn screensaver_command(args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()).unwrap_or("status") {
        "on" | "enable" | "install" => match screensaver_install() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("saturn-art: {e}");
                1
            }
        },
        "off" | "disable" | "uninstall" => match screensaver_remove() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("saturn-art: {e}");
                1
            }
        },
        "start" | "show" | "test" => match screensaver_start() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("saturn-art: {e}");
                1
            }
        },
        "status" => {
            screensaver_status();
            0
        }
        "help" | "-h" | "--help" => {
            screensaver_help();
            0
        }
        other => {
            eprintln!("saturn-art: unknown screensaver action: {other}");
            eprintln!("try `saturn-art screensaver --help`");
            2
        }
    }
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(|s| s.as_str()) == Some("screensaver") {
        std::process::exit(screensaver_command(&raw[1..]));
    }

    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("saturn-art: {e}\n\ntry `saturn-art --help`");
            std::process::exit(2);
        }
    };

    let result = if opts.snapshot.is_some() {
        snapshot(&opts)
    } else {
        run(opts)
    };

    if let Err(e) = result {
        eprintln!("saturn-art: {e}");
        std::process::exit(1);
    }
}
