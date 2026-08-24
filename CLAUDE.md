# CLAUDE.md

Guidance for Claude Code when working in this repository.

## What this is

Rust examples for the Seeed Studio XIAO ESP32-C3 driving e-paper displays through the
Seeed ePaper Driver Board for XIAO, using the [`epdsi`](https://crates.io/crates/epdsi)
driver framework. `no_std`, `esp-hal`, target `riscv32imc-unknown-none-elf`.

## Read BRINGUP.md before debugging any panel

**[BRINGUP.md](BRINGUP.md) is required reading before concluding that a driver or example
is broken.** During initial bring-up, nearly every apparent `epdsi` defect turned out to
be stale panel state. It contains reference timings, known hardware incompatibilities,
and the diagnostics.

The rule that matters most:

> **Power-cycle, run once, do not interrupt, then judge. Connect and disconnect FPCs with
> the board unpowered.**

E-paper retains its last write, and the controller can be left latched busy by an
interrupted run or a hot-swapped panel. The *next* run then hits 60 s busy timeouts and
looks broken. `hard_reset` does not clear it — only removing power does.

## Commands

```bash
cargo build --release --examples          # everything
cargo run --release                        # heartbeat, confirms toolchain
cargo run --release --example <name>       # flash and monitor one example
cargo clippy --release --examples          # lint
```

**Always `--release`.** A debug image is over 5 MB against roughly 750 KB. Do not use
`cargo clippy --all-targets`: it tries to build test targets, which need the `test` crate
and fail on `no_std`.

## Debugging discipline

These were learned expensively here. Apply them before writing a fix.

- **Suspect hardware and panel state before suspecting software.** Running stock Arduino
  GxEPD2 across several XIAO boards twice found in one experiment what hours of driver
  analysis did not.
- **Validate any new diagnostic against a known measurement.** A refresh reported as
  100 ms when the panel demonstrably takes 3891 ms means the instrument is broken, not
  that something interesting happened.
- **Watch the panel, not the log.** A hand-rolled trigger sequence once reported entirely
  plausible timings while never driving the display at all.
- **Never reason from a run that was interrupted**, or from any run after one, until the
  board has been power-cycled.
- **Bisect one variable at a time** and prefer a test that writes bytes straight into the
  buffer, bypassing `embedded-graphics`, when isolating geometry from drawing.

## Conventions

**`esp-hal` is pinned to `=1.0.0`** deliberately — see BRINGUP.md. Do not relax it to a
caret requirement; `esp-bootloader-esp-idf` 0.4.0 declares no `esp-hal` dependency, so
Cargo will resolve an incompatible pair that builds fine and then fails to flash.

**Frame buffers are `static mut`**, not stack arrays. The ESP32-C3's default stack will
not hold a 48,000-byte frame.

**BUSY pull direction follows controller polarity**, chosen so an absent panel reads idle
rather than hanging: `Pull::Down` for the active-HIGH SSD1677/SSD1680/SSD1681,
`Pull::Up` for the active-LOW JD79661/UC8253.

**Pin assignments are fixed by the driver board** and identical in every example:
RST=D0/GPIO2, CS=D1/GPIO3, BUSY=D2/GPIO4, DC=D3/GPIO5, SCK=D8/GPIO8, MOSI=D10/GPIO10.
MISO is unused — e-paper is write-only.

**Examples are ports** of the RP2350 originals in
[`rust-rpico2-discovery`](https://github.com/melastmohican/rust-rpico2-discovery). Keep
everything above `main()` byte-identical where possible: the `epdsi` types, buffers and
`embedded-graphics` drawing are HAL-agnostic, and only board bring-up should differ. That
correspondence is the point — it demonstrates the driver framework's portability.

## Git

Commit directly to `main`. Do not open pull requests for the owner's own changes.
