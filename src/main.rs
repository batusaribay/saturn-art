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
//! * `s` toggle stars
//! * `h` hide/show the hint line
//! * `r` reset

use std::fmt::Write as _;
use std::io::{self, BufWriter, Write};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, event, execute, terminal};

mod color;
mod scene;

use scene::{CHARSETS, Params, V3};

const SPIN_RATE: f32 = 0.30; // radians / second, ~21 s per rotation
// Default view: rings tipped up to the right, with the sun nearly behind the
// camera so the globe stays bright and peachy.  The live camera slowly opens
// and closes the rings around this elevation.
const BASE_TILT_DEG: f32 = 10.0;
const TILT_SWING_DEG: f32 = 0.0;
const TILT_RATE: f32 = 0.052; // ~2 minute cycle
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
    sun: V3,
    stars: bool,
    snapshot: Option<String>,
    cols: usize,
    rows: usize,
    time: f32,
    ascii: bool,
    charset: usize,
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
            sun: SUN,
            stars: true,
            snapshot: None,
            cols: 160,
            rows: 50,
            time: 0.0,
            ascii: false,
            charset: 0,
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
            "--charset" => {
                let v = next("--charset")?;
                o.charset = CHARSETS
                    .iter()
                    .position(|c| c.name == v)
                    .or_else(|| v.parse::<usize>().ok())
                    .filter(|i| *i < CHARSETS.len())
                    .ok_or_else(|| format!("unknown charset: {v}"))?;
            }
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
A live, ray-traced Saturn spinning in colored ASCII art

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
  --tilt DEG       camera elevation, -85..85 deg      (11)
  --azimuth DEG    camera angle around the planet     (10)
  --roll DEG       camera roll around its axis        (12)
  --dist R         camera distance in planet radii    (6.5)
  --no-stars       no star field
  --snapshot PATH  render a single frame and exit
  --cols N         frame width  for --snapshot        (160)
  --rows N         frame height for --snapshot        (50)
  --time SECS      scene time for --snapshot          (0)
  --ascii          with --snapshot -, print plain text

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
  s         stars           h       hide footer
  r         reset"
    );
}

// ---------------------------------------------------------------------------
// terminal plumbing
// ---------------------------------------------------------------------------

struct TermGuard;

impl TermGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Self)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        default(info);
    }));
}

// ---------------------------------------------------------------------------
// live mode
// ---------------------------------------------------------------------------

fn run(opts: Options) -> io::Result<()> {
    install_panic_hook();
    let _guard = TermGuard::enter()?;

    let mut out = BufWriter::with_capacity(1 << 19, io::stdout().lock());

    let frame_target = Duration::from_secs_f32(1.0 / opts.fps as f32);
    let mut last_start = Instant::now();

    let mut t: f32 = 0.0;
    let mut speed = opts.speed;
    let mut paused = false;
    let mut stars = opts.stars;
    let mut hint = true;
    let mut charset = opts.charset;
    let mut status: Option<(String, f32)> = None;
    let mut dims = (0usize, 0usize);
    let mut prev: Option<Vec<scene::Cell>> = None;

    'main: loop {
        let frame_start = Instant::now();
        let dt = (frame_start - last_start).as_secs_f32().min(0.1);
        last_start = frame_start;

        // --- input -------------------------------------------------------
        while event::poll(Duration::ZERO)? {
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
                KeyCode::Char('h') | KeyCode::Char('H') => hint = !hint,
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
            t += dt * speed;
        }

        let (cols, rows) = terminal::size()?;
        // The renderer clamps to a minimum size; keep the indexing in sync.
        let cols = (cols as usize).max(4);
        let rows = (rows as usize).max(3);

        let p = Params {
            tilt: opts.tilt_deg.to_radians() + TILT_SWING_DEG.to_radians() * (t * TILT_RATE).sin(),
            azimuth: opts.azimuth_deg.to_radians(),
            roll: opts.roll_deg.to_radians() + 0.05 * (t * 0.027 + 2.1).sin(),
            distance: opts.distance,
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

        // Overlay the hint on the last row rather than sacrificing a row of
        // the scene: the bottom of the frame is empty sky at normal tilts.
        if let Some((_, left)) = &mut status {
            *left -= dt;
            if *left <= 0.0 {
                status = None;
            }
        }

        if hint {
            let _ = write!(s, "\x1b[{};1H\x1b[38;2;104;104;116m", rows);
            let mut text = String::from("saturn-art");
            if paused {
                text.push_str("   paused");
            }
            match &status {
                Some((msg, _)) => {
                    text.push_str("   ");
                    text.push_str(msg);
                }
                None => text
                    .push_str("   q quit   space pause   +/- speed   c charset   s stars   h hide"),
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

fn main() {
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
