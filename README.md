# saturn-art

A Rust CLI to render ray-traced Saturn

![saturn-art in a terminal](demo.gif)

## Why this exists

A terminal can do more than print tables. This tool ray-traces Saturn into
coloured text, cell by cell, at a frame rate that still leaves the machine
usable. It is a demonstration of how far character cells and 24-bit colour can
go.

## How it works

The camera is a real perspective camera on an angled orbit. Each run starts at a
random pose and eases into a new nearby pose every few seconds, so the shot
never repeats. Framing comes from the ring system at its widest opening, so the
picture fits the terminal at every pose. Each character cell casts four to nine
rays, depending on terminal size, and composites what they hit, which keeps the
rings from flickering and dropping out at the ansae.

The planet is one ray and sphere intersection. Its surface is looked up in
latitude and longitude against a 27-stop palette sampled from real Saturn
images, with fine ribbons, convective swirls, jet edges, storms, and barges
layered on top. Shading runs in linear RGB with a directional sun, soft
terminator, limb darkening, an atmospheric rim, cool ringshine, and a warm glow
where the rings face the equator.

The rings are a ray and plane hit against a radial density profile with the D,
C, B, and A rings, the Cassini Division, the Encke Gap, and ringlets. Opacity
is composited, so the planet shows through the C ring. Shadows are geometric:
the planet casts an umbra across the ring sheet, and the sheet casts a curved
shadow band on the globe. Ring material orbits at Keplerian rates, faster inside
than outside, with a large-scale brightness asymmetry and two dark spokes.

The result is converted to sRGB and mapped by luminance onto a glyph ramp, with
24-bit colour per cell. There is no permanent overlay. A key press reports
itself briefly on the last row and fades, so the scene keeps the whole terminal.

## Usage

```text
saturn-art [OPTIONS] [COMMAND]

Commands:
  screensaver on       Install saturn-art as the Omarchy idle screensaver
  screensaver off      Restore the stock Omarchy screensaver
  screensaver status   Show what is active
  screensaver start    Show it now, any input dismisses
  screensaver delay    Set when it appears
  screensaver timeout  Set when the lock screen takes over

Options:
  --view NAME      edge, classic, dramatic, or closeup (edge)
  --charset NAME   Glyph ramp: classic, detailed, soft, blocks,
                   dense, dots, minimal, or mixed (classic)
  --fps N          Animation frame rate, 5 to 120 (30, 10 as screensaver)
  --speed X        Time multiplier (1.0)
  --tilt DEG       Camera elevation, -85 to 85 degrees (15)
  --azimuth DEG    Camera angle around the planet (10)
  --roll DEG       Camera roll around its axis (12)
  --dist R         Camera distance in planet radii (6.5)
  --zoom X         Framing zoom, 0.5 to 3.0 (1.0)
  --screensaver    Any key or click quits
  --snapshot PATH  Render a single frame and exit
  --cols N         Frame width for --snapshot (160)
  --rows N         Frame height for --snapshot (50)
  --time SECS      Scene time for --snapshot (0)
  --ascii          With --snapshot -, print plain text
  --color WHEN     Color: auto, always, or never (auto)
  -h, --help       Print help
  -V, --version    Print version
```

Typical calls:

```bash
saturn-art
saturn-art --view classic
saturn-art --view closeup --speed 1.5
saturn-art --tilt 2 --roll 18
saturn-art --snapshot saturn.ppm --cols 240 --rows 70 --time 6
saturn-art --snapshot - --ascii --cols 100 --rows 40
```

## Notes

Views: `edge` is rings tipped up to the right with a bright globe; `classic` is
the straight-on postcard; `dramatic` is angled and rolled; `closeup` is a low
camera with strong perspective. Keys: `q` or `Esc` quits, `space` pauses, `+`
and `-` change speed, `c` cycles the glyph ramp, and `r` resets time and speed.

On Omarchy, `saturn-art screensaver on` installs a launcher shim and puts its
directory first in the login PATH. The idle service and the Screensaver menu
then open one fullscreen instance per monitor. `screensaver off` removes the
shim and restores the timers saved when it was installed. `screensaver delay`
and `screensaver timeout` accept `90`, `90s`, `5m`, `1h30m`, `never`, or
`default`; both are stored in `~/.config/omarchy/shell.json`, and
`timeout never` disables the lock screen entirely.

The default spin is about 21 seconds per rotation. Saturn really takes 10.7
hours, so this is a screen-saver pace with the same direction and differential
ring motion. `--speed` goes from 0.05x to 8x. Ring thickness is a physical lie:
the real rings are tens of metres thick and are drawn here as a zero-thickness
sheet.

Rendering scales with the terminal. A 140x45 terminal is comfortable at 30 fps;
large terminals drop to fewer samples per cell. The scene assumes a dark
background. `--snapshot -` writes a PPM to stdout, or plain text with `--ascii`,
which makes it usable from scripts and for wallpapers.

Color uses truecolor when the terminal reports it, falls back to 256 colors and
then 16 colors, and runs without color with `--color never`, `NO_COLOR`,
`CLICOLOR=0`, or `TERM=dumb`. PPM snapshots always keep full color because they
are files, not terminal output.

## Install

**Requirements:** Rust 1.88+, `git`, and a terminal with 24-bit colour.

```bash
cargo install --git https://github.com/batusaribay/saturn-art.git --locked
```

## License

This project is licensed under the [MIT License](LICENSE).
