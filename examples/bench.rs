//! Rough CPU benchmark: 16 held voices of a few factory patches, per engine.
//!
//! cargo run --release --example bench

use std::time::Instant;

use shizuoka7::cartridges;
use shizuoka7::dx7::{Engine, Settings};
use shizuoka7::enums::EngineSw;

fn main() {
    let sr = 48000.0;
    let secs = 20.0;
    for (name, engine) in [("Modern", EngineSw::Modern), ("Mark I", EngineSw::MarkI), ("Vintage", EngineSw::Vintage)] {
        let mut total = 0.0;
        for (cart, voice) in [(cartridges::ROM1A, 10), (cartridges::ROM1A, 16), (cartridges::ROM1A, 0), (cartridges::ROM1A, 14)] {
            let mut e = Engine::new(sr);
            e.apply_settings(&Settings { engine, ..Settings::default() }, true);
            e.set_patch(&cartridges::builtin(cart).voice(voice));
            let n = (secs * sr) as usize;
            let t = Instant::now();
            let mut acc = 0.0f32;
            for i in 0..n {
                // Retrigger a 16-note cluster every half second.
                if i % 24000 == 0 {
                    for k in 0..16 {
                        e.note_off(36 + 3 * k);
                        e.note_on(36 + 3 * k, 100);
                    }
                }
                acc += e.process_sample();
            }
            std::hint::black_box(acc);
            total += t.elapsed().as_secs_f64() / secs;
        }
        println!("{name:8} {:.2} % of one core (16 voices, 48 kHz)", total / 4.0 * 100.0);
    }
}
