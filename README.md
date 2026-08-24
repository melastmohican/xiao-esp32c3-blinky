# xiao-esp32c3-blinky

Rust examples for the [Seeed Studio XIAO ESP32-C3](https://www.seeedstudio.com/Seeed-XIAO-ESP32C3-p-5431.html),
driving e-paper displays through the
[ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html)
with the [`epdsi`](https://crates.io/crates/epdsi) driver framework.

## Why the C3

The XIAO ESP32-C6 does not work with this ePaper Driver Board — the stock Arduino
GxEPD2 sketch produces only noise on the C6, while the same sketch and the same panel
work correctly on the XIAO ESP32-C3, MG24 and nRF52840. The fault is board-level, not
in the driver software, so these examples target the C3.

## Hardware

- **Board:** Seeed Studio XIAO ESP32-C3 (`riscv32imc-unknown-none-elf`)
- **Carrier:** ePaper Driver Board for XIAO, 24-pin FPC
- **Displays verified:** Good Display GDEQ0426T82 (4.26" mono, 800x480),
  GDEM0213B74 (2.13" mono, 122x250), ZJY122250-0213AJH-E5 (2.13" quad-colour, 122x250),
  GDEM0154Z90 (1.54" tri-colour, 200x200)

### Pin mapping

Fixed by the driver board; nothing to wire by hand.

| Signal | XIAO pin | ESP32-C3 GPIO |
| :--- | :--- | :--- |
| RST | D0 | GPIO2 |
| CS | D1 | GPIO3 |
| BUSY | D2 | GPIO4 |
| DC | D3 | GPIO5 |
| SCK | D8 | GPIO8 |
| MOSI | D10 | GPIO10 |

MISO is unused — e-paper is write-only.

Note that these differ from the XIAO ESP32-C6, where the same `D` pins map to
GPIO0/1/2/21/19/18. The physical `D` positions are identical across the XIAO family;
only the underlying GPIO numbers change.

## Running

Build with `--release` — a debug image is over 5 MB against roughly 750 KB, and there is
no reason to flash the larger one.

### Why esp-hal is pinned

`esp-hal` is pinned to exactly `=1.0.0`. `esp-bootloader-esp-idf` 0.4.0 declares no
`esp-hal` dependency, so Cargo will happily resolve it alongside esp-hal 1.1.x — which
changes the image layout. The result builds without complaint but `espflash` then
rejects it:

```
Error: ESP-IDF App Descriptor missing in your `esp-hal` application
```

even though `esp_app_desc!()` is present and `ESP_APP_DESC` is in the binary. Two
combinations are valid:

| esp-hal | esp-bootloader-esp-idf |
| :--- | :--- |
| `1.0.0` | `0.4.0` — used here, matches the C6 project |
| `1.1.x` | `0.5.0` — declares `esp-hal ~1.1`, so Cargo enforces the pairing |

Upgrade both together or neither.

```bash
# Serial heartbeat, confirms toolchain and flashing work
cargo run --release

# Panel demos
cargo run --release --example ssd1677_gdeq0426t82_epd   # 4.26" mono, differential refresh
cargo run --release --example ssd1680_gdem0213b74_epd   # 2.13" mono, banded partial refresh
cargo run --release --example jd79661_zjy122250_epd     # 2.13" quad-colour
cargo run --release --example ssd1681_gdem0154z90_epd   # 1.54" tri-colour (~90 s, do not interrupt)
cargo run --release --example uc8253_gdey037t03_epd     # 3.7" mono -- does NOT work on the C3, see BRINGUP.md

# Diagnostics, for bringing up a new panel
cargo run --release --example epd_diag4        # 4.26": fill black, then white
cargo run --release --example epd_diag_213     # 2.13": fill black/white, then edge stripes
cargo run --release --example epd_diag_partial # 2.13": full vs partial vs banded, timed
```

See **[BRINGUP.md](BRINGUP.md)** for reference timings, diagnostics, and the hardware
findings behind these examples.

### If a panel misbehaves

**Power-cycle the board and let the run finish uninterrupted before drawing any
conclusions.** E-paper retains whatever was last written, and a run cut short mid-write
leaves the panel in a state that makes the *next* run look broken — shifted content,
refreshes that return instantly, or refreshes that appear to hang. Several apparent
driver bugs during bring-up here turned out to be exactly that. The diagnostics above
exist to establish a clean baseline: `epd_diag_213` proves the data path and geometry,
`epd_diag_partial` times full against partial refresh.

Reference timings for the 4.26" and 2.13" mono panels: full refresh ≈3.9 s, partial
refresh ≈1.0 s, and moving a 4,000-byte frame over SPI at 4 MHz ≈8 ms.

The XIAO ESP32-C3 has **no user-controllable onboard LED** (unlike the C6's GPIO15), so
`src/main.rs` prints a counter over USB serial and toggles D6 / GPIO6, where an external
LED can be wired. D6 is clear of every pin the ePaper board uses.

## Licence

MIT OR Apache-2.0.
