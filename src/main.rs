//! saturn-art: a live, realistic Saturn in colored ASCII art.
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
//! * `r` reset

use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufWriter, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, event, execute, terminal};

mod color;
mod config;
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
    snapshot: Option<String>,
    cols: usize,
    rows: usize,
    time: f32,
    ascii: bool,
    charset: usize,
    screensaver: bool,
    fps_set: bool,
    color: ColorMode,
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
            snapshot: None,
            cols: 160,
            rows: 50,
            time: 0.0,
            ascii: false,
            charset: 0,
            screensaver: false,
            fps_set: false,
            color: ColorMode::Auto,
        }
    }
}

fn finite_arg(value: String, name: &str) -> Result<f32, String> {
    let number = value.parse::<f32>().map_err(|_| format!("bad {name}"))?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(format!("bad {name}: expected a finite number"))
    }
}

fn apply_view(o: &mut Options, name: &str) -> Result<(), String> {
    match name {
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
    Ok(())
}

fn parse_charset(value: &str) -> Result<usize, String> {
    CHARSETS
        .iter()
        .position(|charset| charset.name == value)
        .or_else(|| value.parse::<usize>().ok())
        .filter(|index| *index < CHARSETS.len())
        .ok_or_else(|| format!("unknown charset: {value}"))
}

fn apply_config(o: &mut Options, config: config::Config) -> Result<(), String> {
    if let Some(view) = config.view.as_deref() {
        apply_view(o, view)?;
    }
    if let Some(charset) = config.charset.as_deref() {
        o.charset = parse_charset(charset)?;
    }
    if let Some(fps) = config.fps {
        o.fps = fps;
        o.fps_set = true;
    }
    if let Some(speed) = config.speed {
        o.speed = speed;
    }
    if let Some(tilt) = config.tilt {
        o.tilt_deg = tilt;
    }
    if let Some(azimuth) = config.azimuth {
        o.azimuth_deg = azimuth;
    }
    if let Some(roll) = config.roll {
        o.roll_deg = roll;
    }
    if let Some(distance) = config.distance {
        o.distance = distance;
    }
    if let Some(zoom) = config.zoom {
        o.zoom = zoom;
    }
    if let Some(color) = config.color.as_deref() {
        o.color = ColorMode::parse(color).ok_or_else(|| format!("unknown color mode: {color}"))?;
    }
    Ok(())
}

fn parse_args(args: &[String], mut o: Options) -> Result<Options, String> {
    let mut args = args.iter().cloned();

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
                o.fps_set = true;
            }
            "--speed" => {
                o.speed = finite_arg(next("--speed")?, "--speed")?.clamp(0.05, 8.0);
            }
            "--tilt" => {
                o.tilt_deg = finite_arg(next("--tilt")?, "--tilt")?.clamp(-85.0, 85.0);
            }
            "--azimuth" | "--angle" => {
                o.azimuth_deg = finite_arg(next("--azimuth")?, "--azimuth")?.clamp(-360.0, 360.0);
            }
            "--view" => apply_view(&mut o, &next("--view")?)?,
            "--roll" => {
                o.roll_deg = finite_arg(next("--roll")?, "--roll")?.clamp(-180.0, 180.0);
            }
            "--dist" => {
                o.distance = finite_arg(next("--dist")?, "--dist")?.clamp(3.6, 60.0);
            }
            "--zoom" => {
                o.zoom = finite_arg(next("--zoom")?, "--zoom")?.clamp(0.5, 3.0);
            }
            "--charset" => {
                o.charset = parse_charset(&next("--charset")?)?;
            }
            "--screensaver" => o.screensaver = true,
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
                o.time = finite_arg(next("--time")?, "--time")?;
            }
            "--ascii" => o.ascii = true,
            "--color" => {
                let v = next("--color")?;
                o.color = ColorMode::parse(&v).ok_or_else(|| format!("unknown color mode: {v}"))?;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    // A screensaver should sip power: a third of the frame rate and 2x2
    // sampling (see the supersampling threshold) is plenty for a slow orbital
    // drift.  Pass --fps to override.
    if o.screensaver && !o.fps_set {
        o.fps = 10;
    }
    if o.ascii && o.snapshot.as_deref() != Some("-") {
        return Err("--ascii requires --snapshot -".into());
    }
    Ok(o)
}

fn print_help() {
    println!(
        "\
A Rust CLI to render ray-traced Saturn

Usage
  saturn-art                              run it
  saturn-art --snapshot out.ppm           render one frame to a PPM image
  saturn-art --snapshot - --ascii         print a plain-text frame
  saturn-art --help                       this text

Options
  --view NAME      edge | classic | dramatic | closeup
  --charset NAME   glyph ramp (see Charsets)           (classic)
  --fps N          animation frame rate, 5..120       (30, 10 as screensaver)
  --speed X        time multiplier                    (1.0)
  --tilt DEG       camera elevation, -85..85 deg      (15)
  --azimuth DEG    camera angle around the planet     (10)
  --roll DEG       camera roll around its axis        (12)
  --dist R         camera distance in planet radii    (6.5)
  --zoom X         framing zoom, 0.5..3.0             (1.0)
  --screensaver    any key or click quits (used by the Omarchy screensaver)
  --snapshot PATH  render a single frame and exit
  --cols N         frame width  for --snapshot        (160)
  --rows N         frame height for --snapshot        (50)
  --time SECS      scene time for --snapshot          (0)
  --ascii          with --snapshot -, print plain text
  --color WHEN     color: auto, always, or never            (auto)
  -h, --help       this text
  -V, --version    print the version

Screensaver (Omarchy)
  saturn-art screensaver on       install it as the idle screensaver
  saturn-art screensaver off      restore the stock Omarchy screensaver
  saturn-art screensaver status   show what is active
  saturn-art screensaver start    show it now (any input dismisses)
  saturn-art screensaver delay 2m       appear after 2m of idle
  saturn-art screensaver timeout 10m    lock after 10m (never = stay on Saturn)

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
  r         reset"
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
        let mut guard = Self { mouse: false };
        execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        if screensaver {
            let _ = execute!(io::stdout(), event::EnableMouseCapture);
            guard.mouse = true;
        }
        Ok(guard)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorSupport {
    Mono,
    Ansi16,
    Ansi256,
    TrueColor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Paint {
    None,
    True(u8, u8, u8),
    Idx(u8),
    Basic(u8),
}

impl ColorSupport {
    fn detect(mode: ColorMode) -> Self {
        match mode {
            ColorMode::Never => Self::Mono,
            ColorMode::Always => Self::TrueColor,
            ColorMode::Auto => {
                if std::env::var_os("NO_COLOR").is_some()
                    || std::env::var("CLICOLOR").ok().as_deref() == Some("0")
                {
                    return Self::Mono;
                }
                let forced = std::env::var("CLICOLOR_FORCE")
                    .ok()
                    .is_some_and(|value| !value.is_empty() && value != "0");
                if !forced && !io::IsTerminal::is_terminal(&io::stdout()) {
                    return Self::Mono;
                }
                let term = std::env::var("TERM").unwrap_or_default();
                if term == "dumb" {
                    return Self::Mono;
                }
                let colorterm = std::env::var("COLORTERM").unwrap_or_default();
                if colorterm.eq_ignore_ascii_case("truecolor")
                    || colorterm.eq_ignore_ascii_case("24bit")
                {
                    return Self::TrueColor;
                }
                if term.contains("256color") {
                    return Self::Ansi256;
                }
                Self::Ansi16
            }
        }
    }

    fn paint(self, cell: scene::Cell) -> Paint {
        match self {
            Self::Mono => Paint::None,
            Self::TrueColor => Paint::True(cell.r, cell.g, cell.b),
            Self::Ansi256 => Paint::Idx(nearest_256(cell.r, cell.g, cell.b)),
            Self::Ansi16 => Paint::Basic(nearest_16(cell.r, cell.g, cell.b)),
        }
    }

    fn status(self) -> Paint {
        match self {
            Self::Mono => Paint::None,
            Self::TrueColor => Paint::True(120, 120, 132),
            Self::Ansi256 => Paint::Idx(nearest_256(120, 120, 132)),
            Self::Ansi16 => Paint::Basic(90),
        }
    }
}

fn emit_paint(s: &mut String, paint: Paint) {
    match paint {
        Paint::None => {}
        Paint::True(r, g, b) => {
            let _ = write!(s, "\x1b[38;2;{r};{g};{b}m");
        }
        Paint::Idx(index) => {
            let _ = write!(s, "\x1b[38;5;{index}m");
        }
        Paint::Basic(code) => {
            let _ = write!(s, "\x1b[{code}m");
        }
    }
}

fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let level = |value: u8| ((value as u16 * 5 + 127) / 255) as u8;
    16 + 36 * level(r) + 6 * level(g) + level(b)
}

fn nearest_16(r: u8, g: u8, b: u8) -> u8 {
    let bright = ((r as u16 + g as u16 + b as u16) / 3) > 140;
    let base = if r >= g && r >= b {
        31
    } else if g >= r && g >= b {
        32
    } else {
        34
    };
    if bright { base + 60 } else { base }
}

fn run(opts: Options) -> io::Result<()> {
    install_panic_hook();
    let support = ColorSupport::detect(opts.color);
    let _guard = TermGuard::enter(opts.screensaver)?;

    let mut out = BufWriter::with_capacity(1 << 19, io::stdout().lock());

    let frame_target = Duration::from_secs_f32(1.0 / opts.fps as f32);
    let mut last_start = Instant::now();

    let mut t: f32 = 0.0;
    let mut speed = opts.speed;
    let mut paused = false;
    let mut charset = opts.charset;
    let mut status: Option<(String, f32)> = None;
    let mut dims = (0usize, 0usize);
    let mut previous: Option<Vec<scene::Cell>> = None;
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
            ramp: CHARSETS[charset].ramp,
        };
        if dims != (cols, rows) {
            previous = None;
        }
        let cells = scene::render(cols, rows, &p, previous.as_deref());

        let mut s = String::with_capacity(cols * rows * 4 + 64);
        if dims != (cols, rows) {
            // New geometry: wipe whatever the previous layout left behind,
            // and make sure the cursor stays hidden after a window change.
            s.push_str("\x1b[2J");
            dims = (cols, rows);
        }
        s.push_str("\x1b[?25l");

        // Send only cells that changed.  With no star field the sky is a
        // constant black, so most of a large screen costs nothing per frame.
        let mut last_paint = Paint::None;
        let mut cursor: Option<(usize, usize)> = None;
        for j in 0..rows {
            for i in 0..cols {
                let n = j * cols + i;
                let c = cells[n];
                let changed = match previous.as_ref() {
                    None => true,
                    Some(old) => {
                        let o = old[n];
                        o.ch != c.ch || o.r != c.r || o.g != c.g || o.b != c.b
                    }
                };
                if !changed {
                    continue;
                }
                if cursor != Some((i, j)) {
                    let _ = write!(s, "\x1b[{};{}H", j + 1, i + 1);
                }
                if c.ch != b' ' {
                    let paint = support.paint(c);
                    // Reuse the current color when the next cell is within a
                    // couple of levels: imperceptible, and it saves a lot of
                    // escape bytes on smooth gradients.
                    let close = match (last_paint, paint) {
                        (Paint::True(pr, pg, pb), Paint::True(r, g, b)) => {
                            pr.abs_diff(r) <= 2 && pg.abs_diff(g) <= 2 && pb.abs_diff(b) <= 2
                        }
                        (last, next) => last == next,
                    };
                    if !close {
                        emit_paint(&mut s, paint);
                        last_paint = paint;
                    }
                }
                s.push(c.ch as char);
                cursor = if i + 1 < cols { Some((i + 1, j)) } else { None };
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
            let _ = write!(s, "\x1b[{};1H", rows);
            emit_paint(&mut s, support.status());
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

        // The last row carries the fade-out status message, so repaint it
        // every frame: the message then disappears cleanly.
        let mut sent = cells;
        for i in 0..cols {
            sent[(rows - 1) * cols + i].ch = 0;
        }
        previous = Some(sent);

        s.push_str("\x1b[0m");
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
        let stdout = io::stdout();
        let mut out = BufWriter::new(stdout.lock());
        out.write_all(s.as_bytes())?;
        out.flush()?;
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

fn state_home() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/state"))
}

fn idle_state_path() -> PathBuf {
    state_home().join("saturn-art/screensaver.json")
}

fn on_path(command: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v -- \"$1\"")
        .arg("sh")
        .arg(command)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn login_which(command: &str) -> Option<String> {
    let out = Command::new("bash")
        .arg("-lc")
        .arg("command -v -- \"$1\"")
        .arg("bash")
        .arg(command)
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !path.is_empty() {
        Some(path)
    } else {
        None
    }
}

fn strip_block(text: &str) -> io::Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut skip = false;
    let mut found = false;
    for line in text.lines() {
        if line == PATH_BEGIN {
            if skip || found {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "nested or duplicate saturn-art PATH block",
                ));
            }
            skip = true;
            found = true;
            continue;
        }
        if line == PATH_END {
            if !skip {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unmatched saturn-art PATH block end",
                ));
            }
            skip = false;
            continue;
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    if skip {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unterminated saturn-art PATH block",
        ));
    }
    Ok(out)
}

fn write_block(path: &Path, shim_dir: &Path) -> io::Result<()> {
    let old = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut text = strip_block(&old)?;
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
    let _ = write!(
        text,
        "export PATH=\"{}:$PATH\"",
        shell_double_quote(&shim_dir.to_string_lossy())
    );
    text.push('\n');
    text.push_str(PATH_END);
    text.push('\n');
    write_text_atomic(path, &text)
}

/// Escape a value for use inside a double-quoted shell word.
fn shell_double_quote(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '"' | '$' | '`') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn clean_block(path: &Path) -> io::Result<()> {
    let old = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let cleaned = strip_block(&old)?;
    if cleaned != old {
        write_text_atomic(path, &cleaned)?;
    }
    Ok(())
}

fn user_shell_config() -> PathBuf {
    home_dir().join(".config/omarchy/shell.json")
}

fn default_shell_config() -> PathBuf {
    PathBuf::from("/usr/share/omarchy/config/omarchy/shell.json")
}

/// The config the running shell is actually using, when it can tell us.
fn live_shell_config() -> Option<serde_json::Value> {
    let out = Command::new("omarchy-shell")
        .args(["shell", "listShellConfig"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// The user's shell.json, falling back to Omarchy's defaults when it is absent.
fn read_shell_config() -> Option<serde_json::Value> {
    for path in [user_shell_config(), default_shell_config()] {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        if let Ok(config) = serde_json::from_str(&text) {
            return Some(config);
        }
    }
    None
}

fn idle_seconds_from(config: &serde_json::Value, key: &str) -> Option<u64> {
    config.get("idle")?.get(key)?.as_u64()
}

fn idle_seconds(key: &str) -> Option<u64> {
    live_shell_config()
        .or_else(read_shell_config)
        .and_then(|config| idle_seconds_from(&config, key))
}

/// Idle seconds before the screensaver starts (Omarchy default: 150).
fn delay_seconds() -> u64 {
    idle_seconds("screensaver").unwrap_or(150)
}

/// Idle seconds before the lock screen takes over (Omarchy default: 300).
fn lock_seconds() -> u64 {
    idle_seconds("lock").unwrap_or(300)
}

fn describe_timing() -> String {
    match (delay_seconds(), lock_seconds()) {
        (0, 0) => "idle screensaver and lock both disabled".into(),
        (0, lock) => format!("no idle screensaver; lock after {}", format_duration(lock)),
        (delay, 0) => format!("appears after {}; never locks", format_duration(delay)),
        (delay, lock) => format!(
            "appears after {}; locks after {}",
            format_duration(delay),
            format_duration(lock)
        ),
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds == 0 {
        return "never".into();
    }
    let mut parts = Vec::new();
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if seconds > 0 {
        parts.push(format!("{seconds}s"));
    }
    parts.join(" ")
}

/// `90`, `90s`, `5m`, `1h30m`, `never`, `off`, or `default`.
fn parse_duration(text: &str) -> Result<Option<u64>, String> {
    let text = text.trim().to_ascii_lowercase();
    match text.as_str() {
        "default" => return Ok(None),
        "never" | "off" => return Ok(Some(0)),
        "" => return Err("missing a duration".into()),
        _ => {}
    }

    let mut total: u64 = 0;
    let mut digits = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        if ch.is_whitespace() {
            continue;
        }
        let unit = match ch {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            _ => {
                return Err(format!(
                    "unknown unit `{ch}`; use s, m, h, never, or default"
                ));
            }
        };
        if digits.is_empty() {
            return Err(format!("missing a number before `{ch}`"));
        }
        let n: u64 = digits
            .parse()
            .map_err(|_| "duration is too large".to_string())?;
        let seconds = n.checked_mul(unit).ok_or("duration is too large")?;
        total = total.checked_add(seconds).ok_or("duration is too large")?;
        digits.clear();
    }
    if !digits.is_empty() {
        let n: u64 = digits
            .parse()
            .map_err(|_| "duration is too large".to_string())?;
        total = total.checked_add(n).ok_or("duration is too large")?;
    }
    Ok(Some(total))
}

fn apply_idle(config: &mut serde_json::Value, key: &str, value: Option<u64>) -> Result<(), String> {
    let object = config
        .as_object_mut()
        .ok_or("shell.json is not a JSON object")?;
    let empty = {
        let idle = object
            .entry("idle")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or("`idle` in shell.json is not an object")?;
        match value {
            Some(seconds) => {
                idle.insert(key.to_string(), serde_json::json!(seconds));
            }
            None => {
                idle.remove(key);
            }
        }
        idle.is_empty()
    };
    if empty {
        object.remove("idle");
    }
    Ok(())
}

fn load_shell_config_for_write() -> io::Result<serde_json::Value> {
    let path = user_shell_config();
    match fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text)
            .map_err(|e| io::Error::other(format!("could not parse {}: {e}", path.display()))),
        Ok(_) => Ok(read_shell_config().unwrap_or_else(|| serde_json::json!({ "version": 1 }))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(read_shell_config().unwrap_or_else(|| serde_json::json!({ "version": 1 })))
        }
        Err(error) => Err(error),
    }
}

fn write_text_atomic(path: &Path, text: &str) -> io::Result<()> {
    let destination = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let dir = destination.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let permissions = match fs::metadata(&destination) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    for attempt in 0..100 {
        let mut name = std::ffi::OsString::from(".");
        name.push(
            destination
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("config")),
        );
        name.push(format!(".{}.{}.tmp", std::process::id(), attempt));
        let tmp = dir.join(name);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = match options.open(&tmp) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            if let Some(permissions) = permissions.clone() {
                file.set_permissions(permissions)?;
            }
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })();
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
        if let Err(error) = fs::rename(&tmp, &destination) {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary config file",
    ))
}

fn reload_shell() {
    // The shell watches the file, but ask for a reload so `status` is truthful
    // immediately.
    let _ = Command::new("omarchy-shell")
        .args(["-q", "shell", "reloadConfig"])
        .status();
}

fn write_idle(key: &str, value: Option<u64>) -> io::Result<()> {
    let path = user_shell_config();
    let mut config = load_shell_config_for_write()?;
    apply_idle(&mut config, key, value).map_err(io::Error::other)?;
    let text = serde_json::to_string_pretty(&config).map_err(io::Error::other)?;
    write_text_atomic(&path, &(text + "\n"))?;
    reload_shell();
    Ok(())
}

/// Remember the idle timers as they were before saturn-art touched them, so
/// `screensaver off` can put everything back.  Only the first snapshot counts:
/// reinstalling or changing the timers later must not overwrite it.
fn save_idle_snapshot() -> io::Result<()> {
    let path = idle_state_path();
    if path.exists() {
        return Ok(());
    }

    let config_path = user_shell_config();
    let config = match fs::read_to_string(&config_path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let idle = match config.as_deref() {
        Some(text) if text.trim().is_empty() => None,
        Some(text) => {
            let value: serde_json::Value = serde_json::from_str(text).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("could not parse {}: {error}", config_path.display()),
                )
            })?;
            value.get("idle").cloned()
        }
        None => None,
    };
    if idle.as_ref().is_some_and(|idle| !idle.is_object()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("`idle` in {} is not an object", config_path.display()),
        ));
    }
    let snapshot = serde_json::json!({
        "config_existed": config.is_some(),
        "idle": idle,
    });
    let text = serde_json::to_string_pretty(&snapshot).map_err(io::Error::other)?;
    write_text_atomic(&path, &(text + "\n"))
}

fn apply_snapshot(
    config: &mut serde_json::Value,
    snapshot: &serde_json::Value,
) -> Result<(), String> {
    let object = config
        .as_object_mut()
        .ok_or("shell.json is not a JSON object")?;
    match snapshot.get("idle") {
        Some(serde_json::Value::Null) | None => {
            object.remove("idle");
        }
        Some(idle) => {
            if !idle.is_object() {
                return Err("saved idle settings are not an object".into());
            }
            object.insert("idle".to_string(), idle.clone());
        }
    }
    Ok(())
}

fn restore_idle_snapshot() -> io::Result<()> {
    let path = idle_state_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let snapshot: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| io::Error::other(format!("could not parse {}: {e}", path.display())))?;

    let config_path = user_shell_config();
    let mut config = load_shell_config_for_write()?;
    apply_snapshot(&mut config, &snapshot).map_err(io::Error::other)?;

    // If saturn-art created shell.json, remove it again once it holds nothing
    // but the idle settings we added.
    let created_by_us = snapshot.get("config_existed") == Some(&serde_json::Value::Bool(false))
        && config
            .as_object()
            .is_some_and(|object| object.keys().all(|key| key == "version" || key == "idle"));
    if created_by_us
        && !config
            .as_object()
            .is_some_and(|object| object.contains_key("idle"))
    {
        match fs::remove_file(&config_path) {
            Ok(()) => reload_shell(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    } else {
        let text = serde_json::to_string_pretty(&config).map_err(io::Error::other)?;
        write_text_atomic(&config_path, &(text + "\n"))?;
        reload_shell();
    }

    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let _ = fs::remove_dir(state_home().join("saturn-art"));
    Ok(())
}

fn screensaver_delay(arg: Option<&str>) -> i32 {
    let Some(arg) = arg else {
        if delay_seconds() == 0 {
            println!("The idle screensaver is disabled.");
        } else {
            println!(
                "saturn-art appears after {} of idle.",
                format_duration(delay_seconds())
            );
        }
        return 0;
    };

    let value = match parse_duration(arg) {
        Ok(value) => value,
        Err(e) => {
            eprintln!("saturn-art: {e}");
            eprintln!("try `saturn-art screensaver --help`");
            return 2;
        }
    };
    if let Err(e) = save_idle_snapshot() {
        eprintln!("saturn-art: could not save the previous timing: {e}");
        return 1;
    }
    if let Err(e) = write_idle("screensaver", value) {
        eprintln!("saturn-art: {e}");
        return 1;
    }

    match value {
        Some(0) => {
            println!("The idle screensaver is disabled; nothing starts automatically.");
            println!("Turn it back on with `saturn-art screensaver delay default`.");
        }
        Some(_) => println!(
            "saturn-art now appears after {} of idle.",
            format_duration(delay_seconds())
        ),
        None => println!(
            "saturn-art appears after Omarchy's default ({} of idle).",
            format_duration(delay_seconds())
        ),
    }
    0
}

fn screensaver_timeout(arg: Option<&str>) -> i32 {
    let Some(arg) = arg else {
        if lock_seconds() == 0 {
            println!("The lock screen never takes over; saturn-art stays up until input.");
        } else {
            println!(
                "The lock screen takes over after {} of idle.",
                format_duration(lock_seconds())
            );
            let delay = delay_seconds();
            if delay > 0 && lock_seconds() > delay {
                println!(
                    "  that leaves {} of saturn-art.",
                    format_duration(lock_seconds() - delay)
                );
            }
        }
        return 0;
    };

    let value = match parse_duration(arg) {
        Ok(value) => value,
        Err(e) => {
            eprintln!("saturn-art: {e}");
            eprintln!("try `saturn-art screensaver --help`");
            return 2;
        }
    };
    if let Err(e) = save_idle_snapshot() {
        eprintln!("saturn-art: could not save the previous timing: {e}");
        return 1;
    }
    if let Err(e) = write_idle("lock", value) {
        eprintln!("saturn-art: {e}");
        return 1;
    }

    match value {
        Some(0) => {
            println!("The lock screen is now disabled; saturn-art stays up until input.");
            println!(
                "The machine will not lock automatically (`omarchy system lock` still works)."
            );
            println!("Restore it with `saturn-art screensaver timeout default`.");
        }
        Some(_) => {
            let delay = delay_seconds();
            let lock = lock_seconds();
            println!(
                "The lock screen takes over after {} of idle.",
                format_duration(lock)
            );
            if delay > 0 && lock > delay {
                println!(
                    "  that leaves {} of saturn-art.",
                    format_duration(lock - delay)
                );
            } else if delay > 0 {
                println!(
                    "  note: the screensaver starts after {}, so the lock fires first.",
                    format_duration(delay)
                );
            }
        }
        None => println!(
            "The lock screen takes over after Omarchy's default ({} of idle).",
            format_duration(lock_seconds())
        ),
    }
    0
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

    // Remember the timers before anything can change them, so `off` restores
    // them even if this is the first install.
    save_idle_snapshot()?;

    let shim = shim_path();
    let dir = shim.parent().unwrap_or(Path::new(".")).to_path_buf();
    fs::create_dir_all(&dir)?;
    write_text_atomic(&shim, SCREENSAVER_LAUNCHER)?;
    #[cfg(unix)]
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755))?;

    // Login shells are what Omarchy's idle service and menu invoke the
    // launcher through, so the PATH block belongs in ~/.bash_profile.
    write_block(&home_dir().join(".bash_profile"), &dir)?;
    // Older versions used ~/.bashrc; that lands before the mise env and is
    // overwritten, so drop it.
    clean_block(&home_dir().join(".bashrc"))?;

    println!("saturn-art is now the Omarchy screensaver.");
    println!("  show it now:  saturn-art screensaver start");
    println!("  turn it off:  saturn-art screensaver off  (restores your idle timing)");
    println!("  timing:       {}", describe_timing());
    match login_which("omarchy-launch-screensaver").as_deref() {
        Some(p) if Path::new(p) == shim => {}
        _ => println!(
            "warning: a login shell does not resolve the shim first; check the PATH block in ~/.bash_profile"
        ),
    }
    Ok(())
}

fn screensaver_remove() -> io::Result<()> {
    let had_snapshot = idle_state_path().exists();

    let shim = shim_path();
    match fs::remove_file(&shim) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(dir) = shim.parent() {
        let _ = fs::remove_dir(dir);
    }
    let _ = fs::remove_dir(data_home().join("saturn-art"));

    clean_block(&home_dir().join(".bash_profile"))?;
    clean_block(&home_dir().join(".bashrc"))?;

    restore_idle_snapshot()?;

    println!("The Omarchy screensaver is back to the stock one.");
    if had_snapshot {
        println!("Your idle timing is back to how it was before `saturn-art screensaver on`.");
    }
    if lock_seconds() == 0 {
        println!(
            "note: the idle lock is still disabled; run `saturn-art screensaver timeout default` to restore it."
        );
    }
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
    let delay = delay_seconds();
    let lock = lock_seconds();
    println!(
        "  appears:          {}",
        if delay == 0 {
            "never (idle screensaver disabled)".to_string()
        } else {
            format!("after {} of idle", format_duration(delay))
        }
    );
    println!(
        "  lock screen:      {}",
        if lock == 0 {
            "never (stays up until input)".to_string()
        } else if delay > 0 && lock > delay {
            format!(
                "after {} of idle ({} of saturn-art)",
                format_duration(lock),
                format_duration(lock - delay)
            )
        } else {
            format!("after {} of idle", format_duration(lock))
        }
    );
    if lock == 0 {
        println!("  note:             the machine will not lock automatically");
    }
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
  saturn-art screensaver on           install it as the idle screensaver
  saturn-art screensaver off          restore the stock Omarchy screensaver
  saturn-art screensaver status       show what is active
  saturn-art screensaver start        show it now (any input dismisses)

Timing
  saturn-art screensaver delay 2m         appear after 2 minutes of idle
  saturn-art screensaver delay never      never start automatically
  saturn-art screensaver timeout 10m      lock screen after 10 minutes
  saturn-art screensaver timeout never    stay until input (no auto-lock)

Durations: 90, 90s, 5m, 1h30m, never (0), or default.  Stored in
~/.config/omarchy/shell.json as idle.screensaver and idle.lock.  `on` saves
the values you had before, and `off` restores them.

The shim is installed at ~/.local/share/saturn-art/bin and a marked PATH
block goes into ~/.bash_profile, because Omarchy runs the screensaver
launcher through a login shell."
    );
}

fn screensaver_command(args: &[String]) -> i32 {
    let action = args.first().map(|s| s.as_str()).unwrap_or("status");
    let valid_arity = match action {
        "on" | "enable" | "install" | "off" | "disable" | "uninstall" | "start" | "show"
        | "test" | "help" | "-h" | "--help" => args.len() == 1,
        "status" => args.len() <= 1,
        "delay" | "timeout" | "lock" => args.len() <= 2,
        _ => true,
    };
    if !valid_arity {
        eprintln!("saturn-art: wrong number of arguments for screensaver {action}");
        eprintln!("try `saturn-art screensaver --help`");
        return 2;
    }
    match action {
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
        "delay" => screensaver_delay(args.get(1).map(String::as_str)),
        "timeout" | "lock" => screensaver_timeout(args.get(1).map(String::as_str)),
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

    let asks_for_help = raw
        .iter()
        .take_while(|argument| argument.as_str() != "--")
        .any(|argument| matches!(argument.as_str(), "-h" | "--help" | "-V" | "--version"));
    let cfg = if asks_for_help {
        config::Config::default()
    } else {
        config::load().unwrap_or_else(|error| {
            eprintln!("saturn-art: {error}");
            std::process::exit(2);
        })
    };
    let mut initial = Options::default();
    if let Err(error) = apply_config(&mut initial, cfg) {
        eprintln!("saturn-art: {error}");
        std::process::exit(2);
    }
    let opts = match parse_args(&raw, initial) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizes_256_color_cube() {
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(255, 0, 0), 196);
    }

    #[test]
    fn escapes_shell_sensitive_characters_in_paths() {
        assert_eq!(
            shell_double_quote(r#"/home/me/$data/"quoted"/`x`"#),
            r#"/home/me/\$data/\"quoted\"/\`x\`"#
        );
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("90"), Ok(Some(90)));
        assert_eq!(parse_duration("90s"), Ok(Some(90)));
        assert_eq!(parse_duration("2m"), Ok(Some(120)));
        assert_eq!(parse_duration("1h"), Ok(Some(3600)));
        assert_eq!(parse_duration("1h30m"), Ok(Some(5400)));
        assert_eq!(parse_duration("1h 2m 3s"), Ok(Some(3723)));
        assert_eq!(parse_duration("never"), Ok(Some(0)));
        assert_eq!(parse_duration("off"), Ok(Some(0)));
        assert_eq!(parse_duration("0"), Ok(Some(0)));
        assert_eq!(parse_duration("default"), Ok(None));
        assert_eq!(parse_duration(" DEFAULT "), Ok(None));
        assert!(parse_duration("5x").is_err());
        assert!(parse_duration("m").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("-5m").is_err());
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(0), "never");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(150), "2m 30s");
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(5400), "1h 30m");
    }

    #[test]
    fn merges_idle_without_touching_other_settings() {
        let mut config = serde_json::json!({
            "version": 1,
            "idle": { "screensaver": 150, "lock": 300 },
            "bar": { "background": "glassy" }
        });

        apply_idle(&mut config, "lock", Some(0)).unwrap();
        assert_eq!(config["idle"]["lock"], 0);
        assert_eq!(config["idle"]["screensaver"], 150);
        assert_eq!(config["bar"]["background"], "glassy");

        apply_idle(&mut config, "lock", None).unwrap();
        assert!(config["idle"].get("lock").is_none());
        assert_eq!(config["idle"]["screensaver"], 150);

        apply_idle(&mut config, "screensaver", None).unwrap();
        assert!(config.get("idle").is_none());
        assert_eq!(config["bar"]["background"], "glassy");
    }

    #[test]
    fn creates_idle_from_an_empty_config() {
        let mut config = serde_json::json!({});
        apply_idle(&mut config, "screensaver", Some(60)).unwrap();
        assert_eq!(config["idle"]["screensaver"], 60);
    }

    #[test]
    fn restores_the_saved_idle_object() {
        let mut config = serde_json::json!({
            "version": 1,
            "idle": { "screensaver": 60, "lock": 0 },
            "bar": { "background": "glassy" }
        });
        let snapshot = serde_json::json!({
            "config_existed": true,
            "idle": { "lock": 300, "screensaver": 150 }
        });

        apply_snapshot(&mut config, &snapshot).unwrap();
        assert_eq!(config["idle"], snapshot["idle"]);
        assert_eq!(config["bar"]["background"], "glassy");
    }

    #[test]
    fn restores_a_missing_idle_object() {
        let mut config = serde_json::json!({
            "version": 1,
            "idle": { "screensaver": 60 }
        });

        apply_snapshot(&mut config, &serde_json::json!({ "idle": null })).unwrap();
        assert!(config.get("idle").is_none());
        assert_eq!(config["version"], 1);
    }

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn rejects_non_finite_numeric_arguments() {
        for option in [
            "--speed",
            "--tilt",
            "--azimuth",
            "--roll",
            "--dist",
            "--zoom",
            "--time",
        ] {
            for value in ["NaN", "inf", "-inf"] {
                let result = parse_args(&arguments(&[option, value]), Options::default());
                assert!(result.is_err(), "{option} accepted {value}");
            }
        }
        assert!(
            parse_args(
                &arguments(&["--time", &f32::MAX.to_string()]),
                Options::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn ascii_snapshots_require_stdout() {
        assert!(parse_args(&arguments(&["--ascii"]), Options::default()).is_err());
        assert!(
            parse_args(
                &arguments(&["--snapshot", "out.ppm", "--ascii"]),
                Options::default()
            )
            .is_err()
        );
        assert!(
            parse_args(
                &arguments(&["--snapshot", "-", "--ascii"]),
                Options::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_duration_overflow() {
        assert!(parse_duration(&format!("{}h", u64::MAX)).is_err());
        assert!(parse_duration(&format!("{}s1", u64::MAX)).is_err());
    }

    #[test]
    fn rejects_malformed_managed_path_blocks() {
        let valid = format!("before\n{PATH_BEGIN}\nexport PATH=x\n{PATH_END}\nafter\n");
        assert_eq!(strip_block(&valid).unwrap(), "before\nafter\n");
        assert!(strip_block(&format!("before\n{PATH_BEGIN}\nlost")).is_err());
        assert!(strip_block(&format!("before\n{PATH_END}\nafter")).is_err());
        assert!(strip_block(&format!("{PATH_BEGIN}\n{PATH_BEGIN}\n{PATH_END}")).is_err());
    }

    #[test]
    fn rejects_invalid_idle_snapshot_types() {
        let mut config = serde_json::json!({ "version": 1 });
        assert!(apply_snapshot(&mut config, &serde_json::json!({ "idle": 42 })).is_err());
        assert!(config.get("idle").is_none());
    }

    #[test]
    fn rejects_extra_screensaver_arguments_before_acting() {
        for values in [
            &["status", "extra"][..],
            &["on", "extra"][..],
            &["delay", "1m", "extra"][..],
        ] {
            assert_eq!(screensaver_command(&arguments(values)), 2);
        }
    }

    fn test_directory(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("saturn-art-{name}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn atomic_write_replaces_content_and_preserves_permissions() {
        let directory = test_directory("atomic");
        let path = directory.join("config");
        fs::write(&path, "old").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        write_text_atomic(&path, "new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_symlinks_and_ignores_stale_temp_links() {
        use std::os::unix::fs::symlink;

        let directory = test_directory("atomic-links");
        let target = directory.join("target");
        let path = directory.join("config");
        let victim = directory.join("victim");
        fs::write(&target, "old").unwrap();
        fs::write(&victim, "safe").unwrap();
        symlink(&target, &path).unwrap();
        symlink(
            &victim,
            directory.join(format!(".config.{}.0.tmp", std::process::id())),
        )
        .unwrap();

        write_text_atomic(&path, "new").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(fs::read_to_string(&victim).unwrap(), "safe");
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
