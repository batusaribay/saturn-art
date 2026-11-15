//! saturn-art — a live, realistic Saturn in colored ASCII art.
//!
//! Usage:
//!   saturn-art [--fps N] [--speed X] [--tilt DEG] [--no-stars]
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

use scene::{Params, V3};

const SPIN_RATE: f32 = 0.135; // radians / second, ~46 s per rotation
const BASE_TILT_DEG: f32 = 26.0;
const SUN: V3 = V3::new(-0.58, 0.46, -0.67);

// ---------------------------------------------------------------------------
// options
// ---------------------------------------------------------------------------

struct Options {
    fps: u32,
    speed: f32,
    tilt_deg: f32,
    stars: bool,
    snapshot: Option<String>,
    cols: usize,
    rows: usize,
    time: f32,
    ascii: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            fps: 30,
            speed: 1.0,
            tilt_deg: BASE_TILT_DEG,
            stars: true,
            snapshot: None,
            cols: 160,
            rows: 50,
            time: 0.0,
            ascii: false,
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
                o.tilt_deg = o.tilt_deg.clamp(-89.0, 89.0);
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
saturn-art {} — a live, realistic Saturn in colored ASCII art

USAGE
  saturn-art                        run it
  saturn-art --help                 this text
  saturn-art --snapshot out.ppm     render one frame to a PPM image

OPTIONS
  --fps N          animation frame rate, 5..120       (30)
  --speed X        time multiplier                    (1.0)
  --tilt DEG       ring opening angle, -89..89 deg    (26)
  --no-stars       no star field
  --snapshot PATH  render a single frame and exit
  --cols N         frame width  for --snapshot        (160)
  --rows N         frame height for --snapshot        (50)
  --time SECS      scene time for --snapshot          (0)
  --ascii          with --snapshot -, print plain text

KEYS
  q / Esc   quit            space   pause
  + / -     speed           s       toggle stars
  h         hide hint       r       reset

Rendered with one ray per character cell: a latitude/longitude atmosphere
model (belted butterscotch, cream equatorial zone, bluish polar hood), a
26.7-degree ring plane with C/B/A ring densities, the Cassini and Encke
gaps, plus the planet's shadow on the rings and the rings' shadow on the
planet.",
        env!("CARGO_PKG_VERSION")
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
                KeyCode::Char(' ') => paused = !paused,
                KeyCode::Char('+') | KeyCode::Char('=') => speed = (speed * 1.25).min(8.0),
                KeyCode::Char('-') | KeyCode::Char('_') => speed = (speed / 1.25).max(0.2),
                KeyCode::Char('s') | KeyCode::Char('S') => stars = !stars,
                KeyCode::Char('h') | KeyCode::Char('H') => hint = !hint,
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    t = 0.0;
                    speed = opts.speed;
                    paused = false;
                }
                _ => {}
            }
        }

        if !paused {
            t += dt * speed;
        }

        let (cols, rows) = terminal::size()?;
        let (cols, rows) = (cols as usize, rows as usize);
        if cols == 0 || rows == 0 {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }

        let hint_rows = if hint && rows >= 12 { 1 } else { 0 };
        let draw_rows = rows - hint_rows;

        let p = Params {
            tilt: (opts.tilt_deg.to_radians() + 0.020 * (t * 0.11).sin()),
            spin: t * SPIN_RATE,
            sun: SUN,
            time: t,
            stars,
        };
        let cells = scene::render(cols, draw_rows, &p);

        let mut s = String::with_capacity(cols * draw_rows * 8 + 64);
        let mut last_rgb = (0u8, 0u8, 0u8);
        for j in 0..draw_rows {
            if j == 0 {
                s.push_str("\x1b[1;1H");
            } else {
                let _ = write!(s, "\x1b[{};1H", j + 1);
            }
            for i in 0..cols {
                let c = cells[j * cols + i];
                // Spaces need no escape sequence at all.
                if c.ch == b' ' {
                    s.push(' ');
                    continue;
                }
                if (c.r, c.g, c.b) != last_rgb {
                    let _ = write!(s, "\x1b[38;2;{};{};{}m", c.r, c.g, c.b);
                    last_rgb = (c.r, c.g, c.b);
                }
                s.push(c.ch as char);
            }
        }

        if hint_rows == 1 {
            let _ = write!(s, "\x1b[{};1H\x1b[38;2;104;104;116m", rows);
            let mut text = String::from("saturn-art");
            if paused {
                text.push_str("   paused");
            } else if (speed - 1.0).abs() > 0.01 {
                let _ = write!(text, "   {speed:.2}x");
            }
            text.push_str("   q quit   space pause   +/- speed   s stars   h hide");
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
        spin: opts.time * SPIN_RATE,
        sun: SUN,
        time: opts.time,
        stars: opts.stars,
    };
    let cells = scene::render(opts.cols, opts.rows, &p);

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
