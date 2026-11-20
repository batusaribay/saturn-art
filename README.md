# saturn-art

A Rust CLI that renders ray-traced Saturn

![saturn-art in a terminal](demo.gif)

The GIF above is a compressed preview. It looks much better in a real terminal,
at full resolution, in 24-bit color, and in motion.

## How it works

1. The camera is a real perspective camera on an angled orbit. Each run starts
   at a random pose — ring opening, orbit angle and roll — and every few
   seconds it picks a new nearby pose and eases into it, so the shot never
   repeats. The framing is computed from the ring system at its widest
   opening, so the picture is the same size at every pose and always fits the
   terminal. Glyph hysteresis and temporal smoothing only cross two solid
   cells, so moving edges leave no ghosts and the razor-thin rings do not
   strobe.
2. Each character cell casts four to nine rays, depending on the terminal
   size, and composites what they hit. This is what keeps the rings from
   flickering and dropping out at the ansae.
3. The planet is one ray/sphere intersection. Its surface is looked up in
   latitude and longitude against a 27-stop palette sampled from real Saturn
   images: peach-cream equatorial zone, tan belts, mauve-brown polar regions.
   Fine ribbons, convective swirls, meandering jet edges, white storm ovals,
   and dark barges are layered on top and rotate with the planet.
4. Shading is done in linear RGB: a directional sun, soft terminator, limb
   darkening, a thin atmospheric rim, a little cool ringshine so the night side
   keeps its silhouette, and a warm glow where the rings face the equator.
5. The rings are a ray/plane hit against a radial density profile with the
   D, C, B, and A rings, the Cassini Division, the Encke Gap, and extra
   ringlets. Opacity is composited, so the planet shows through the C ring.
6. Shadows are geometric, not painted. The planet casts a real umbra across the
   ring sheet, and the ring sheet casts a curved shadow band on the globe.
7. The ring material orbits: an angular density pattern rotates at Keplerian
   rates, faster inside than outside, with a large-scale brightness asymmetry
   and two dark spokes that sweep around the disc.
8. The result is converted to sRGB and mapped by luminance onto a glyph ramp,
   with 24-bit color per cell. Eight ramps are built in — classic, detailed,
   soft, blocks, dense, dots, minimal and mixed — and `c` cycles them live.
9. There is no permanent overlay. A key press reports itself briefly on the
   last row (charset, speed, pause) and then fades, so the scene keeps the
   whole terminal.

## Usage

### Views

| View | Look |
| --- | --- |
| `saturn-art` | Randomised ring opening, orbit and roll at a fixed size (default). |
| `saturn-art --view classic` | Straight-on postcard, rings open, no roll. |
| `saturn-art --view dramatic` | Angled and rolled, near rings larger. |
| `saturn-art --view closeup` | Low camera, strong perspective, rings wrap the globe. |

### Keys

| Key | Action |
| --- | --- |
| `q` / `Esc` | Quit. |
| `space` | Pause. |
| `+` / `-` | Speed up / slow down. |
| `c` | Cycle the glyph ramp. |
| `r` | Reset time and speed. |

### Options

```text
--view NAME      edge | classic | dramatic | closeup
--charset NAME   glyph ramp                         (classic)
--fps N          animation frame rate, 5..120       (30, 10 as screensaver)
--speed X        time multiplier                    (1.0)
--tilt DEG       camera elevation, -85..85 deg      (15)
--azimuth DEG    camera angle around the planet     (10)
--roll DEG       camera roll around its axis        (12)
--dist R         camera distance in planet radii    (6.5)
--zoom X         framing zoom, 0.5..3.0             (1.0)
--screensaver    any key or click quits (idle screensaver)
--snapshot PATH  render a single frame and exit
--cols N         frame width  for --snapshot        (160)
--rows N         frame height for --snapshot        (50)
--time SECS      scene time for --snapshot          (0)
--ascii          with --snapshot -, print plain text
```

### Examples

```bash
saturn-art
saturn-art --view classic
saturn-art --view closeup --speed 1.5
saturn-art --charset detailed
saturn-art --charset dots --view edge

saturn-art --tilt 2           # nearly edge-on
saturn-art --roll 18           # steeper diagonal
saturn-art --dist 4            # stronger perspective

saturn-art --snapshot saturn.ppm --cols 240 --rows 70 --time 6
saturn-art --snapshot - --ascii --cols 100 --rows 40
```

`--snapshot -` writes a PPM to stdout, or plain text with `--ascii`, which makes
it usable from scripts and for wallpapers.

## Install

**Requirements:** Rust 1.85+, `git`, and a terminal with 24-bit color.

```bash
cargo install --git https://github.com/batusaribay/saturn-art.git --locked
```

## Omarchy screensaver

On Omarchy, saturn-art can take over the idle screensaver. It is built into
the tool:

```bash
saturn-art screensaver on       # make saturn-art the screensaver
saturn-art screensaver off      # restore the stock one
saturn-art screensaver status   # show what is active
saturn-art screensaver start    # show it now (any input dismisses)
```

After upgrading, run `make install` so the installed binary (the one the
screensaver launches) is the new build.

`screensaver on` installs a launcher shim at
`~/.local/share/saturn-art/bin/omarchy-launch-screensaver` and puts that
directory first in the login PATH (a marked block in `~/.bash_profile`).
Omarchy's idle service and the System > Screensaver menu both invoke
`omarchy-launch-screensaver` through a login shell, so both now open one
fullscreen saturn-art per monitor on the usual screensaver workspace. Any
key, click or scroll dismisses it, exactly like the stock screensaver, and
it honours `omarchy toggle screensaver` and the `screensaver-off` flag.

The launcher does not use Omarchy's stock screensaver terminal profile (the
18pt black one made for the TTE text effect). It starts your terminal with
its normal font, padding, colors and theme, so the screensaver looks like
running `saturn-art` yourself, just fullscreen. Font size is what controls
detail: at 11pt a 1080p screen gives roughly 218x61 cells, at 18pt only
133x37. Change the font in your terminal's config if you want it finer or
coarser. If your theme is translucent, the desktop shows through it, same as
a normal window.

### Timing

Two idle timers decide how it behaves, both stored in
`~/.config/omarchy/shell.json`:

- `idle.screensaver` — when Saturn appears (Omarchy default: 150 s).
- `idle.lock` — when the lock screen takes over and closes it (default: 300 s).

Both are settable from the tool:

```bash
saturn-art screensaver timeout 10m      # lock after 10 minutes of idle
saturn-art screensaver timeout never    # never lock; stays until input
saturn-art screensaver timeout default  # drop the override

saturn-art screensaver delay 2m         # appear after 2 minutes of idle
saturn-art screensaver delay never      # never start automatically
saturn-art screensaver delay default
```

Durations accept `90`, `90s`, `5m`, `1h30m`, `never` (same as `0`), or
`default`. `timeout` is measured from when you stop using the machine, so
the time actually spent on Saturn is `timeout - delay`; `status` prints that
number for you.

`timeout never` disables the lock screen entirely: the display will not lock
or turn off on its own. `omarchy system lock` and your lock keybind still
work, and `screensaver off` puts the timer back to the value you had before.

To disable the idle screensaver without removing the shim, use
`saturn-art screensaver delay never`, or:

```bash
omarchy toggle screensaver
```

The screensaver is tuned to be cheap: 10 fps by default (`--fps` overrides
it), 2x2 supersampling on large grids, and after the first frame only cells
that actually changed are written. A 1080p fullscreen run costs about a
fifth of one core, plus a few percent in the terminal. The mouse pointer is
hidden while it is up and restored when the last window closes.

`screensaver off` removes the shim and the PATH block, returning Omarchy to
its stock screensaver. It also restores the idle timers exactly as they were
before `screensaver on` — the values are saved to
`~/.local/state/saturn-art/screensaver.json` when the screensaver is
installed, and deleted after they are put back.

## Uninstall

```bash
cargo uninstall saturn-art
```

## Notes

- The default spin is about 21 seconds per rotation. Saturn really takes 10.7
  hours; this is a screen-saver pace with the same direction and differential
  ring motion. `--speed` goes from 0.05x to 8x.
- Ring thickness is a physical lie: the real rings are tens of meters thick.
  They are drawn as a zero-thickness sheet, which is why the ansae get thin.
- Rendering scales with the terminal. A 140x45 terminal is comfortable at
  30 fps; very large terminals automatically drop to fewer samples per cell.
- The scene assumes a dark terminal background. In a light theme it still
  works, but the sky will not be black.

## License

This project is licensed under the [MIT License](LICENSE).
