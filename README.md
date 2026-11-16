# saturn-art

A live, ray-traced Saturn spinning in colored ASCII art

![saturn-art in a terminal](demo.gif)

A higher-quality MP4 of the same scene is in [demo.mp4](demo.mp4).

## What it is

A single Rust binary that renders Saturn in the terminal in real time. Every
character is a ray cast through a 3D model of the planet, its atmosphere, and
its rings, shaded in linear light and quantized to a character ramp. There are
no sprites and no pre-rendered frames: the ring shadow, the terminator, and the
perspective stay consistent while the globe turns.

It runs in the terminal you already have, needs no assets, and works over SSH.
Point it at the sky and leave it running.

## Why this exists

Terminal art is usually either a static ASCII drawing or a loop of pre-made
frames. Saturn is a good argument for doing the real thing instead: the rings
are a thin sheet seen almost edge-on, so their shadow, their near/far
perspective, and the planet's shadow on them all depend on where the sun and
the camera are. Animating that by hand is a geometry problem, not an art
problem. Casting a ray per cell solves it once and then lets you move the
camera, change the ring opening, or watch a storm rotate without any new art.

## How it works

1. The camera is a real perspective camera on an angled orbit. It is placed
   just 2.5 degrees above the ring plane, so the rings read as a thin bright
   blade tipped up to the right, and the near ansa is noticeably larger than
   the far one.
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
8. The result is converted to sRGB and mapped by luminance onto
   `" .:-=+*#%@"`, with 24-bit color per cell. A hash-based star field twinkles
   behind everything.
9. A small footer line lists the keys on the last row, over empty sky, so the
   scene keeps the whole terminal.

## Usage

### Views

| View | Look |
| --- | --- |
| `saturn-art` | Near edge-on rings tipped up to the right (default). |
| `saturn-art --view classic` | Straight-on postcard, rings open, no roll. |
| `saturn-art --view dramatic` | Angled and rolled, near rings larger. |
| `saturn-art --view closeup` | Low camera, strong perspective, rings wrap the globe. |

### Keys

| Key | Action |
| --- | --- |
| `q` / `Esc` | Quit. |
| `space` | Pause. |
| `+` / `-` | Speed up / slow down. |
| `s` | Toggle the star field. |
| `h` | Hide or show the footer. |
| `r` | Reset time and speed. |

### Options

```text
--view NAME      edge | classic | dramatic | closeup
--fps N          animation frame rate, 5..120       (30)
--speed X        time multiplier                    (1.0)
--tilt DEG       camera elevation, -85..85 deg      (2.5)
--azimuth DEG    camera angle around the planet     (10)
--roll DEG       camera roll around its axis        (12)
--dist R         camera distance in planet radii    (6.5)
--no-stars       no star field
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

saturn-art --tilt 1.2          # even more edge-on
saturn-art --roll 18           # steeper diagonal
saturn-art --dist 4            # stronger perspective

saturn-art --snapshot saturn.ppm --cols 240 --rows 70 --time 6
saturn-art --snapshot - --ascii --cols 100 --rows 40
```

`--snapshot -` writes a PPM to stdout, or plain text with `--ascii`, which makes
it usable from scripts and for wallpapers.

## Installation

**Requirements:** Rust 1.85+, `git`, and a terminal with 24-bit color.

```bash
cargo install --git https://github.com/batusaribay/saturn-art.git --locked
```

## Uninstall

```bash
cargo uninstall saturn-art
```

## Notes

- The default spin is about 29 seconds per rotation. Saturn really takes 10.7
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
