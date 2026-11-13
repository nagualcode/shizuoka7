# DX7 hardware model: decisions and evidence

This file records what Shizuoka7's DX7 VINTAGE engine changes relative to Dexed,
why, and how sure we are. Full research notes with every source and
confidence tag: [`RESEARCH.md`](RESEARCH.md).

Tags: **[V]** verified from a primary source (die analysis, firmware,
patent, bit-accurate emulator source), **[M]** measured on hardware,
**[I]** inferred, **[S]** speculation.

## Implemented

| # | Change | Evidence | Code |
|---|---|---|---|
| 1 | Core at 9.4265 MHz / 192 = 49,096.35 Hz, resampled to host | [V] service manual, Shirriff, VDX7 | `vintage::DX7_SAMPLE_RATE`, `resample.rs` |
| 2 | Carrier-count compensation COM = 0, 1024, 1664, 2048, 2432, 2688 (1024 = 6 dB) added to carrier attenuation before exp | [V] Shirriff part 4, VDX7 algorithm ROM, US 4,554,857 | `core.rs` `COM`, `render_mki` |
| 3 | Per-voice DAC: 15-bit voice word, mask 3/2/1/0 LSBs for abs >= 4096/2048/1024/below. Word scale: core value >> 12 (the core's 2^26 full scale is the chip's 16384; `gain = env12 << 2` and the +-4-cycle modulation depth both fix this) | [V] Shirriff part 5, VDX7; scale [I] | `synth::dx7_dac`, `synth::render` |
| 4 | Voices summed after conversion (analog S&H sum) | [V] Shirriff part 5, US 4,632,001 | `synth::render` |
| 5 | Sample-and-hold: two hold circuits alternate over the 16 voice slots, so a voice is held for 2/16 sample (droop 0.02 dB at 15 kHz, not a full-sample hold's 1.4 dB) | [V] Shirriff part 5 (SH1/SH2 per voice slot), [I] width | `resample::HOLD` |
| 6 | 5th-order output low-pass: pole 6788 Hz; pairs 11588 Hz / Q 1.017, 16062 Hz / Q 4.13 | [V] topology, [I] values from VDX7's circuit model (component values not readable in available scans) | `vintage::output_filter` (complex), embedded in the resampler kernel with its causal phase |
| 7 | Pitch grid 1024/octave; key code 0 / 85 / 170 steps within each minor third; firmware coarse table, fine = round(4096 log2(1+f/100)), `>>2`; fixed mode `(0x16AC + FIXED + 136 fine) >> 2` | [V] ajxs firmware v1.8 disassembly | `note::osc_freq_hw` |
| 8 | LFO / modulation update every ~132 samples (firmware 372 Hz tick) | [V] firmware timer | `synth::render` (`lfo_steps`) |
| 9 | DAC ladder gain error (Tolerance) | [I] 0.1 % resistors per VDX7 | `Engine::apply_settings` |
| 10 | Constant op-amp hiss about -90 dBFS | [S] no measurement published | `vintage::HISS` |
| 11 | Per-sample EGS envelope: 12-bit attenuation, global clock, rate masks 0xAA/0xEA/0xEE/0xFE, attack jump to 0x94C with slope `(att >> 8) + 2`, 479-step delay quirk with stage-0 compression | [V] MSFA measurements, [I] VDX7 `Envelope` (delay compression is VDX7's speculation) | `env::EgsEnv` |
| 12 | Converter noise through the analog scaler: each voice slot passes the DAC / I-V noise at its range gain (/1../8), idle slots at /8 | [V] scaler position (Shirriff part 5), [S] level | `synth::Block::dac_noise`, `vintage::DAC_NOISE` |
| 13 | DAC bit-weight errors (major-carry step at the zero crossing), with the ladder error under Tolerance | [I] R-2R converter behaviour, BA9221 class linearity; the error pattern itself is invented [S] | `Engine::set_dac_errors` |

Level calibration: with carrier scaling a voice never exceeds 16,384 DAC
units; that maps to 0.5 (-6 dBFS), and a level-99 carrier peaks at 8192,
which makes a typical two-carrier voice as loud as in Mark I / Dexed. Six-carrier organ voices are quieter
than in Dexed, as on the hardware.

Why the output filter lives in the resampler: its poles (11.6 and
16.1 kHz, Q 4.13) are close to the host Nyquist. A bilinear-transform IIR
at 48 kHz measures -7.6 dB at 15 kHz instead of +0.8 dB. The kernel is
the inverse transform of the circuit's complex transfer function (times
the hold and an anti-image taper), so `tests::output_filter_matches_dx7_response`
holds within 0.5 dB at 44.1, 48 and 96 kHz, and the phase is the analog
filter's: the 16 kHz ringing follows a transient
(`tests::output_filter_rings_after_not_before`). The whole path between
DAC and jack is linear, so this kernel is the exact response of the
circuit; an explicit component-level simulation would add CPU and no
sound. The non-linear parts (converter, scaler noise) are modelled where
they sit, before it.

## Kept from Dexed (deliberately)

- **Detune law**: Dexed's curve fitted to measurements of a real DX7 [M]
  (±12.7 cents at C2 to ±2.5 at C7 for detune ±7). VDX7 assumes a constant
  ±1 step per unit [I]. The measurement wins until better data appears.
- **Feedback shift**: Dexed's `>> (9 - FB)`, which the Dexed author tuned
  by ear against his DX7. VDX7 uses one bit more. Unresolved [I].
- **Envelope levels, rate scaling, amplitude modulation**: Dexed's
  mappings feed the EGS envelope (AM is applied as an attenuation offset
  held per block). VDX7's `AM << AMS` law and rate-scaling table differ
  slightly [I]; unresolved.

## Open / next

1. VDX7's `AM << AMS` amplitude modulation and firmware rate scaling.
2. 32-level velocity quantisation (`TABLE_MIDI_VEL`).
3. Pitch EG at the firmware's 186 Hz step rate.
4. Sub-sample time offsets between voice slots (order 0, 8, 4, 12, ...).
5. A recording of a real DX7 (sine at several levels, algorithm 32 feedback,
   idle and playing noise floor) to settle the feedback shift, noise
   levels and output gain.
