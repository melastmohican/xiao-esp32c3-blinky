# E-paper bring-up notes

Hard-won findings from getting these panels working. Read this before concluding that
a driver or an example is broken — most apparent bugs here were not.

## The single most important rule

**Power-cycle, run once, do not interrupt, then judge.**

E-paper retains its last write, and the controller can be left *latched busy* if a run is
cut off mid-update. When that happens the next run's refresh hits the driver's 60 s busy
timeout — three times per refresh on SSD1680, which does power-on / update / power-off —
so a single refresh can take three minutes. It is indistinguishable from a hang, and it
is self-perpetuating: a timed-out refresh leaves the panel equally stuck, so the run
after that fails too.

`hard_reset` does **not** clear this. Only removing power does.

During initial bring-up this produced, in order: shifted and clipped content, refreshes
returning in 10 ms, refreshes appearing to hang, and a diagnostic that reported nine
fabricated "100 ms" measurements. None were code defects. All were the same stale-state
problem being measured over and over.

**Also: only connect or disconnect an FPC with the board unpowered.** Hot-swapping a
panel leaves it in an undefined state and is a common way into the situation above.

## Reference timings

Measured on a XIAO ESP32-C3 at 4 MHz SPI. If a run deviates wildly from these, suspect
panel state before suspecting code.

| Panel | Controller | Full refresh | Partial refresh | Notes |
| :--- | :--- | ---: | ---: | :--- |
| GDEQ0426T82 4.26" mono | SSD1677 | ~3745 ms | ~1000 ms | 48,000-byte frame |
| GDEM0213B74 2.13" mono | SSD1680 | ~3891 ms | ~1017 ms | banded partial, fast LUT |
| ZJY122250 2.13" quad | JD79661 | several s | n/a | colour: no fast waveform |
| GDEM0154Z90 1.54" tri | SSD1681 | **~14 s** | **~14 s** | see below |
| GDEY037T03 3.7" mono | UC8253 | — | — | **does not work on the C3**, see below |

Data transfer is never the bottleneck: a 4,000-byte frame takes **8 ms** at 4 MHz, and
drawing a band (bitmap loops, text, rectangles) takes **~17 ms**.

### Colour panels are slow, and that is correct

Tri-colour and quad-colour panels have **no fast waveform**. The coloured pigment is a
heavier particle needing the full OTP waveform to migrate, so every update takes seconds.
On the 1.54" tri-colour panel that is ~14 s per refresh — including the "partial" ones,
which are partial only in that the RAM window is narrowed, not in speed. The full
tri-colour example runs six refreshes and takes about **90 seconds**. This is normal.

Selecting a `Partial` refresh mode on a colour panel is worse than useless: the fast LUT
only exists for monochrome, so it is slow *and* discards the colour plane.

## Diagnostics

Run these before changing any code. They establish whether the fault is real.

```bash
cargo run --release --example epd_diag_213     # 2.13": black, white, then edge stripes
cargo run --release --example epd_diag4        # 4.26": black, then white
cargo run --release --example epd_diag_partial # 2.13": full vs partial vs banded, timed
```

`epd_diag_213`'s stripe test writes **directly into the byte buffer**, bypassing
`embedded-graphics` entirely, so it isolates the RAM window and stride from the drawing
layer. If the stripes land correctly, geometry is fine and any offset is above that.

Two lessons about diagnostics themselves:

- **Validate the instrument against a known measurement.** A diagnostic reporting 100 ms
  for a refresh that is known to take 3891 ms is broken, not informative.
- **Watch the panel, not the log.** A hand-rolled trigger sequence once reported
  plausible timings while never driving the display at all. The display not changing is
  what exposed it.

## Hardware notes

### XIAO ESP32-C6 does not work with the ePaper Driver Board

Confirmed with stock Arduino GxEPD2 across four XIAO variants, same adapter and panel:

| Board | Result |
| :--- | :--- |
| XIAO ESP32-C3 | works perfectly |
| XIAO MG24 | works |
| XIAO nRF52840 | works, some ghosting |
| **XIAO ESP32-C6** | **noise only** |

Since known-good third-party code fails the same way, this is board-level and not a
software problem. Use the C3. The `D` pin positions are identical across the XIAO family
but the GPIOs behind them are not — see the pin table in `README.md`.

### GDEY037T03 3.7" does not work on the XIAO ESP32-C3

Confirmed with both `epdsi` and stock Arduino GxEPD2:

| Board | Result |
| :--- | :--- |
| XIAO MG24 | works |
| XIAO nRF52840 | works |
| **XIAO ESP32-C3** | **panel never responds** |

On the C3 the panel is demonstrably present and alive — BUSY is driven, not floating,
and asserts correctly (active-LOW) in response to a hardware reset. But it never acts on
SPI commands: `refresh` returns in 0 ms and the display never changes. Sweeping the SPI
clock from 4 MHz down to 100 kHz changes nothing, so signal integrity on SCK is not the
cause. Arduino failing the same way rules out the driver.

Two untested candidates remain, both needing instrumentation to separate: power delivery
(the UC8253's DC-DC booster sagging the C3's rail — the same failure mode as the EXT3-1
J3 jumper problem), or something about this panel's FPC that the C3's pin assignment does
not tolerate.

Note also that the 3.7" is **not on Seeed's supported panel list** for this driver board.
Their catalogue covers 1.54", 2.13", 2.9", 4.2", 4.26", 5.65", 5.83" and 7.5". This is an
undocumented combination rather than a defect.

The example is kept in the repository for use on a board that works.

### BUSY pull direction follows controller polarity

Set the pull so a **missing or unpowered panel reads idle**, not busy — otherwise an
absent panel presents as a hang rather than a blank screen.

| Controller | BUSY polarity | Pull |
| :--- | :--- | :--- |
| SSD1677, SSD1680, SSD1681 | active-HIGH | `Pull::Down` |
| JD79661, UC8253 | active-LOW | `Pull::Up` |

### Panel identification

Retail stickers differ; flex ribbons do not. The 2.13" quad-colour panel sold as Good
Display `GDEY0213F51`, Seeed 5779 and Adafruit 6373/6366 is one product — units from
different vendors carry the same `FPC-J002` ribbon and are physically identical. The
2.13" mono `GDEM0213B74` carries `FPC-7528B`.

## Toolchain

`esp-hal` is pinned to exactly `=1.0.0`. `esp-bootloader-esp-idf` 0.4.0 declares **no**
`esp-hal` dependency, so Cargo will happily resolve it alongside esp-hal 1.1.x, which
changes the image layout. The result builds without complaint, then `espflash` rejects it:

```
Error: ESP-IDF App Descriptor missing in your `esp-hal` application
```

even though `esp_app_desc!()` is present and `ESP_APP_DESC` is in the binary. Valid
pairings are esp-hal `1.0.0` with esp-bootloader `0.4.0`, or esp-hal `1.1.x` with
esp-bootloader `0.5.0` — which does declare the constraint. Upgrade both or neither.

Build with `--release`: a debug image is over 5 MB against roughly 750 KB.
