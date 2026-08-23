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
- **Display:** Good Display GDEQ0426T82, 4.26" monochrome, 800x480

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

# Fill the panel black, then white — the simplest end-to-end check
cargo run --release --example epd_diag4

# Full demo: logos, differential refresh, cleanup pass
cargo run --release --example ssd1677_gdeq0426t82_epd
```

The XIAO ESP32-C3 has **no user-controllable onboard LED** (unlike the C6's GPIO15), so
`src/main.rs` prints a counter over USB serial and toggles D6 / GPIO6, where an external
LED can be wired. D6 is clear of every pin the ePaper board uses.

## Licence

MIT OR Apache-2.0.
