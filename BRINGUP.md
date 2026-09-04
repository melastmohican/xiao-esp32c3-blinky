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

### Flash first, power-cycle second

On the XIAO, power-up **is** program-start, so "power-cycle, then run" cannot be done in that
order: replugging immediately runs the *previous* binary, and flashing then interrupts it
mid-refresh — leaving the panel latched exactly as the power-cycle was meant to prevent. The
naive ritual is worse than none.

The order that works:

```bash
cargo run --release --example <name>   # flash. Ignore this run's output; the panel is dirty.
# unplug USB, wait a few seconds, plug back in
espflash monitor                       # attach while the binary is still in its startup hold
```

Examples that matter for this should hold a few seconds before touching the panel, so there is a
window to attach the monitor. `epd_diag_352` holds 8 s. **Only a run watched after the replug
began from a cleanly powered panel** — the one flashing produced does not count.

## Reference timings

Measured on a XIAO ESP32-C3 at 4 MHz SPI. If a run deviates wildly from these, suspect
panel state before suspecting code.

| Panel | Controller | Full refresh | Partial refresh | Notes |
| :--- | :--- | ---: | ---: | :--- |
| GDEQ0426T82 4.26" mono | SSD1677 | ~3745 ms | ~1000 ms | 48,000-byte frame |
| GDEM0213B74 2.13" mono | SSD1680 | ~3891 ms | ~1017 ms | banded partial, fast LUT |
| ZJY122250 2.13" quad | JD79661 | several s | n/a | colour: no fast waveform |
| GDEM0154Z90 1.54" tri | SSD1681 | **~14 s** | **~14 s** | see below |
| GDEY037T03 3.7" mono | UC8253 | works | n/a (partial-window demo only) | works — on a replacement C3. The original C3 was faulty; see below |
| SE0352N14 3.52" tri | UC8253 | ~17369 ms | n/a | works — on a replacement C3. The original C3 was faulty; see below |

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
cargo run --release --example epd_diag_352     # 3.52": ONE refresh, bands. Do not loop this panel.
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

**Two upstream reports match this, and they give different causes.** Both are worth knowing before
concluding anything about the C6:

- [Seeed-Studio/platform-seeedboards#46](https://github.com/Seeed-Studio/platform-seeedboards/issues/46)
  — a Seeed contributor attributes C6 e-paper failures to `GPIO0`–`GPIO7` being **LP_GPIO**, where
  Arduino's `pinMode`/`digitalWrite` fail with *"IO x is not set as GPIO"* because `gpio_config()`
  never registers the pin with periman. On the XIAO ESP32-C6, `D0`/`D1`/`D2` are `GPIO0`/`GPIO1`/
  `GPIO2` — exactly the RST, CS and BUSY the driver board hardwires. That would produce "noise
  only", and it is an **Arduino-core bug**, so it need not apply to `esp-hal`. Our C6 evidence is
  Arduino-only, so this weakens the "board-level, not software" claim above.
- The [ePaper Driver Board forum thread](https://forum.seeedstudio.com/t/epaper-driver-board-for-seeed-studio-xiao/288004)
  — a user saw "scattered dots / garbled pixels" on **five** v2 boards with a C6, where v1 boards
  worked. Seeed traced it to a defective early batch (`MOA250113001`) and replaced them with
  `MOA250210012`. Check the batch sticker before blaming silicon.

Note also that Seeed's compatibility list for this board covers SAMD21, RP2040, nRF52840, ESP32C3
and ESP32S3 — the **C6 is not on it**.

### GDEY037T03 3.7" — history: believed not to work on the XIAO ESP32-C3

**Resolved, like the 3.52" panel: it works.** Retested on the replacement C3 with the
`uc8253_gdey037t03_epd` example — full refresh, six partial-window logo-swap updates, and
a full-waveform cleanup pass, all confirmed correct on the panel. The original C3 was
faulty; the failures documented below were never a controller or driver problem. Everything
in this section describes debugging against that faulty board and is kept for the record.

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

**Re-tested after the 3.52" work, and it still fails.** That is a useful negative: it closes a
hypothesis rather than opening one.

The 3.52" panel, same UC8253 and same board, produced the *same* "refresh returns in 0 ms" symptom,
and that turned out to be three driver bugs — a missing per-refresh `POWER_ON`, a BUSY poll that
ran before the controller asserted, and a 2 ms reset pulse that latched only intermittently. So
the 0 ms symptom was never proof that a panel ignores SPI; it means **BUSY was never observed
asserted**. Since GxEPD2's busy wait has the same race shape, "Arduino fails identically" was
weaker evidence than it read as, and the whole finding deserved re-examination.

It was re-examined. `epdsi` now waits for the BUSY edge after `DISPLAY_REFRESH` on this variant,
and the 3.7" still does not run on the C3. **The race was not the cause here.** Two further things
are now also excluded:

- **Not the SPI clock.** Swept 4 MHz down to 100 kHz, no change.

The 3.52" no longer distinguishes this panel either: **both UC8253 panels now fail on the C3**,
with `epdsi` and with Waveshare's own reference demo. See the section below — this is one fault,
not two, and it belongs to the controller rather than to the 3.7" specifically.

Note also that the 3.7" is **not on Seeed's supported panel list** for this driver board.
Their catalogue covers 1.54", 2.13", 2.9", 4.2", 4.26", 5.65", 5.83" and 7.5". It works
anyway, on a sound C3.

### A refresh can return in 0 ms because BUSY has not asserted yet

Found on the C3 with the 3.52" tri-colour panel, and it is the sharpest instance yet of "the
instrument lied". Serial output from one run:

```
Refresh returned after 0 ms          <- first refresh
Refresh returned after 17268 ms      <- second refresh
```

Both refreshes "completed". The panel showed streaked noise, and — the giveaway — **every run
displayed the previous run's image**.

The panel does not assert BUSY the instant it receives `DISPLAY_REFRESH`, and the host may still
be draining the SPI FIFO when the command call returns. Poll BUSY in that window and it reads
idle, so the driver concludes the refresh finished. It has not: it runs for ~17 s in the
background. Anything written next lands in controller RAM *mid-update*, so the panel drives from
a buffer changing underneath it, and the frame you asked for only appears on the following run.

There are two causes, and both had to be fixed:

- **The controller drops its charge pump after an update.** A bare `DISPLAY_REFRESH` on the next
  frame is silently ignored, so BUSY never asserts at all. Waveshare's reference driver hides this
  by re-running its entire init — which begins with `POWER_ON` — before *every* display operation,
  exactly one refresh per init. `epdsi` now issues `POWER_ON` at the start of each refresh.
- **BUSY assertion is not instant.** A fixed settling delay cannot be tuned reliably: a 10 ms guard
  was observed holding on some refreshes and missing on others in the same run. `epdsi` now waits
  for the BUSY *edge* (`SpiBusWrapper::wait_busy_assert`) after `DISPLAY_REFRESH`, bounded by a
  timeout so a genuinely absent panel still falls through instead of hanging. After `POWER_ON` it
  uses a fixed 100 ms settle instead, matching the reference — `POWER_ON` frequently does not
  assert BUSY on this panel at all.
- **The RST low pulse was too short.** Waveshare's driver uses 2 ms; SID's, for the same panel
  family, uses 30 ms. At 2 ms the reset latched only sometimes, and a reset that does not take
  leaves the controller ignoring everything. `epdsi` now uses 30 ms.

### Resolved: the original C3 board was faulty, not the C3+UC8253 combo

Everything below this point was measured on the original C3 and concluded the controller
combination itself was broken. That conclusion was wrong. Reading 2, flagged below as
"testable and untested," was the answer: **this particular C3 board had degraded.**

A replacement XIAO ESP32-C3 was fitted and re-run through the same sequence recommended
below:

1. `epd_diag_213` on the known-good `GDEM0213B74` (2.13" mono) — 3892 ms, matching the
   ~3891 ms reference exactly. New board is sound.
2. `epd_diag_352` on `SE0352N14` (3.52" tri, UC8253), single-shot — **17369 ms, `OK`**,
   matching the original ~17.3 s measurement almost exactly. Panel showed white | black |
   red with "PLANE TEST" upright, as expected.
3. `uc8253_gdey037t03_epd` on `GDEY037T03` (3.7" mono, UC8253) — full refresh, six
   partial-window logo-swap updates, and a full-waveform cleanup pass, all confirmed
   correct on the panel.

So the UC8253 controller works fine on the C3, on both panels tested. The three `epdsi`
driver fixes found while chasing this (missing per-refresh `POWER_ON`, BUSY-edge wait, 30 ms
reset pulse) were real bugs worth having, and this board's failure obscured that they had
already fixed it.

### History: why "the UC8253 panels do not work on the C3" was believed

Confirmed with two independent driver stacks against the same assembly — same ePaper Driver
Board, same panel, same FPC seating, only the XIAO swapped in the socket:

| Stack | XIAO MG24 | XIAO ESP32-C3 |
| :--- | :--- | :--- |
| `epdsi` (this repo) | works | fails |
| **Waveshare reference Arduino demo** | works | fails |

The Waveshare demo is the strongest evidence available: it is the vendor's own code, written
against this exact controller. GxEPD2 does **not** support the 3.52" `SE0352N14`, so an earlier
note here claiming GxEPD2 confirmation for that panel was wrong; the reference demo replaces it.

That settles attribution. The fault is in the **C3 + ePaper Driver Board + UC8253** combination,
not in `epdsi`. **Stop changing driver code for this.** The three `epdsi` fixes found while
chasing it were real bugs worth having, but they were never going to make the C3 work.

It also matches the C6 result recorded above: this driver board is fussy about which XIAO sits
on it.

**Panel current is no longer the leading candidate.** The 4.26" `GDEQ0426T82` (SSD1677, 800x480 —
the largest frame in the set) and the tri-colour 1.54" `GDEM0154Z90` (SSD1681) both work on the C3
through the *same six pins*. The failure tracks the **controller**, not panel size or colour, which
is the opposite of what a supply-current ceiling looks like. Bulk capacitance and a LiPo on the
driver board's battery connector are therefore low-priority experiments, not the next one.

Note also that the driver board carries no 3.3 V regulator of its own — the datasheet lists only
an `ETA9740` charge IC, and the panel is fed from the XIAO's 3V3 pin either way. A battery does
not bypass that rail.

**Unexplained regression, and it matters.** This same 3.52" panel was measured here at a correct
~17.3 s full refresh on the C3. That is no longer reproducible: the C3 now fails with both stacks.
Two readings of that are open, and they lead to different remedies:

1. The C3 never reliably drove UC8253, and the earlier ~17.3 s readings came from the confounded
   over-refresh sequence recorded below rather than from a sound baseline.
2. **This particular C3 board has degraded** over the course of this work.

Reading 2 is testable and untested: put a known-good panel (the 2.13" `GDEM0213B74`, or the 4.26")
on this C3 and run `epd_diag_213`. If that also fails, the board is damaged and "the C3 cannot
drive UC8253" is the wrong conclusion. Do this before buying another board. A second C3 would
settle it outright.

**Settled: reading 2 was correct.** See "Resolved" above — a replacement C3 drove both the
known-good 2.13" and the SE0352N14 successfully, matching prior reference timings. The original
board was damaged.

**Open question, unresolved:** two of the C3's six pins are strapping pins — `GPIO2` (RST) and
`GPIO8` (SCK) — which the ROM touches before firmware runs. MG24's `PC00`/`PA03` have no
equivalent. That could latch the controller before `init()` executes. Counter-evidence: the
SSD16xx panels drive the same two pins without trouble. Recorded so nobody re-derives it.

### Using the MG24 means giving up Rust

**The MG24 drives both UC8253 panels easily — but it is not a usable target for this repository.**
Silicon Labs' EFR32MG24 has no maintained Rust HAL, and flashing it needs a J-Link rather than the
USB Serial/JTAG the ESP32 parts expose. The MG24 result is a *diagnostic*, not a migration path.

The panel-compatibility matrix and the Rust-viable-board matrix are not the same matrix:

| Board | Drives UC8253 | Rust HAL | Flash without a probe |
| :--- | :--- | :--- | :--- |
| XIAO MG24 | yes | none | no (J-Link) |
| XIAO nRF52840 | yes | `embassy-nrf` | UF2 / serial DFU |
| XIAO ESP32-C3 | **no** | `esp-hal` | yes, USB Serial/JTAG |
| XIAO ESP32-S3 | untested | `esp-hal` (Xtensa, needs espup) | yes, USB Serial/JTAG |
| XIAO RP2350 | untested | `rp235x-hal`, stable toolchain | yes, picotool/UF2 |

Seeed's own compatibility list for this driver board covers SAMD21, RP2040, nRF52840, ESP32C3 and
ESP32S3 — it does not include the MG24 or the C6, despite the MG24 working.

### Do not loop a panel rated for daily updates

Waveshare specify **at least 180 s between refreshes**, and at least one refresh every 24 h — the
same figures appear across their panel manuals. A diagnostic written to measure a failure rate ran
seven full refreshes roughly **19 s** apart, about ten times the rated rate. The result:

- Early iterations rendered cleanly, ~17.3 s each.
- Around the fifth, refreshes began failing and streaked bands appeared.
- The *following* run produced nothing but bands from the start.

That is the test degrading the panel, not the panel exposing a fault. Continuous full refreshes
heat the panel and repeatedly drive the pigment in the same direction; neither is something a
driver change compensates for. Leave it unpowered for several hours — over-refresh degradation is
usually recoverable.

**This invalidates the "5/7 failure rate" that was recorded here.** The failures clustered *late*
in each run, which reads as cumulative degradation rather than the random intermittent fault it
was taken for. Any conclusion drawn from those runs — including that the residual failures pointed
at power delivery — is confounded and has to be re-established one refresh at a time.

`epd_diag_352` is now single-shot for this reason. To judge reliability, run it once, let the
panel rest, run it again, and tally by hand.

More generally: **a repeat-count instrument is only valid if the thing under test tolerates
repetition.** Check the duty cycle before building a loop around any panel.

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

### epdsi 0.1.7

Bumped from 0.1.5 with no code changes — 0.1.6 only deprecated `EpdPanel::vcom()` /
`custom_lut()` / `gate_voltage()` (unused here) and 0.1.7 adds SSD1677 RAM auto-fill,
a hardware-pattern-generator clear path enabled by default. The sibling RP2350 repo
confirmed the same: their bump was a version-string change only.

RAM auto-fill was reverified on the replacement C3 with `epd_diag4` against the
4.26" `GDEQ0426T82`: 3746 ms / 3747 ms for black then white, matching the ~3745 ms
reference, panel confirmed visually. This is the one code path in 0.1.7 the epdsi
changelog itself flags as having no vendor reference driver behind it, so it was
worth a real re-check rather than trusting the build succeeding.

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
