use snellar_core::model::Fx;

const BANDS: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
const MIN_AMOUNT: f64 = 0.05;
const REVERB_DELAYS: [f64; 4] = [43.0, 71.0, 113.0, 181.0];
const REVERB_DECAYS: [f64; 4] = [0.55, 0.42, 0.32, 0.22];

fn n(v: f64) -> String {
    format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
}

fn active(v: f64) -> bool {
    v.abs() > 1e-3
}

fn taps(values: &[f64], scale: f64) -> String {
    values.iter().map(|v| n(v * scale)).collect::<Vec<_>>().join("|")
}

fn bounded(value: f64, neutral: f64, low: f64, high: f64) -> f64 {
    if value.is_finite() { value.clamp(low, high) } else { neutral }
}

pub fn clamp(fx: &Fx) -> Fx {
    Fx {
        preamp: bounded(fx.preamp, 0.0, -24.0, 24.0),
        eq: fx.eq.map(|g| bounded(g, 0.0, -24.0, 24.0)),
        bass: bounded(fx.bass, 0.0, -20.0, 20.0),
        speed: bounded(fx.speed, 1.0, 0.25, 4.0),
        reverb: bounded(fx.reverb, 0.0, 0.0, 1.0),
        echo: bounded(fx.echo, 0.0, 0.0, 1.0),
        stereo: bounded(fx.stereo, 1.0, 0.0, 4.0),
        ..fx.clone()
    }
}

const LOUDNORM: &str = "loudnorm=I=-14:TP=-1.5:LRA=11";

pub fn gain(loudness_db: f64) -> f64 {
    (-loudness_db).min(0.0)
}

pub fn chain(fx: &Fx, measure: bool) -> String {
    let mut stages = if fx.enabled { effects(&clamp(fx)) } else { Vec::new() };
    if measure {
        stages.push(LOUDNORM.into());
    }
    if stages.is_empty() {
        return String::new();
    }
    format!("lavfi=[{}]", stages.join(","))
}

fn effects(fx: &Fx) -> Vec<String> {
    let mut stages = Vec::new();
    if active(fx.preamp) {
        stages.push(format!("volume={}dB", n(fx.preamp)));
    }
    for (freq, gain) in BANDS.iter().zip(fx.eq) {
        if active(gain) {
            stages.push(format!("equalizer=f={freq}:t=o:w=1:g={}", n(gain)));
        }
    }
    if active(fx.bass) {
        stages.push(format!("bass=g={}", n(fx.bass)));
    }
    if active(fx.stereo - 1.0) {
        stages.push(format!("extrastereo=m={}", n(fx.stereo)));
    }
    if fx.echo >= MIN_AMOUNT {
        let amount = fx.echo;
        stages.push(format!("aecho=0.8:0.9:{}:{}", n(280.0 + 220.0 * amount), n(0.7 * amount)));
    }
    if fx.reverb >= MIN_AMOUNT {
        let amount = fx.reverb;
        let delays = taps(&REVERB_DELAYS, 1.0);
        let decays = taps(&REVERB_DECAYS, amount);
        stages.push(format!("aecho={}:0.88:{delays}:{decays}", n(1.0 - 0.25 * amount)));
    }
    if fx.normalize {
        stages.push("dynaudnorm".into());
    }
    stages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_is_empty() {
        assert_eq!(chain(&Fx::default(), false), "");
    }

    #[test]
    fn disabled_is_empty() {
        let fx = Fx { enabled: false, bass: 6.0, ..Fx::default() };
        assert_eq!(chain(&fx, false), "");
    }

    #[test]
    fn stages_in_order() {
        let mut fx = Fx { preamp: -3.0, bass: 4.5, stereo: 1.5, normalize: true, ..Fx::default() };
        fx.eq[0] = 3.0;
        fx.eq[9] = -2.5;
        assert_eq!(
            chain(&fx, false),
            "lavfi=[volume=-3dB,equalizer=f=31:t=o:w=1:g=3,equalizer=f=16000:t=o:w=1:g=-2.5,bass=g=4.5,extrastereo=m=1.5,dynaudnorm]"
        );
    }

    #[test]
    fn reverb_scales_with_amount() {
        let fx = Fx { reverb: 0.5, ..Fx::default() };
        assert_eq!(chain(&fx, false), "lavfi=[aecho=0.88:0.88:43|71|113|181:0.28|0.21|0.16|0.11]");
    }

    fn decays(chain: &str) -> Vec<f64> {
        chain
            .split("aecho=")
            .skip(1)
            .flat_map(|stage| {
                let stage = stage.split([',', ']']).next().unwrap();
                stage.rsplit(':').next().unwrap().split('|').map(|d| d.parse::<f64>().unwrap()).collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn echo_and_reverb_never_get_a_zero_decay() {
        for step in 0..=1000 {
            let amount = step as f64 / 1000.0;
            let chain = chain(&Fx { reverb: amount, echo: amount, ..Fx::default() }, false);
            assert!(decays(&chain).iter().all(|d| *d > 0.0), "amount {amount}: {chain}");
        }
    }

    #[test]
    fn hostile_values_are_clamped() {
        let wild = Fx { preamp: f64::NAN, bass: 1e9, speed: 0.0, stereo: -5.0, reverb: 7.0, echo: -1.0, eq: [99.0; 10], ..Fx::default() };
        let tame = clamp(&wild);
        assert_eq!(
            (tame.preamp, tame.bass, tame.speed, tame.stereo, tame.reverb, tame.echo, tame.eq[0]),
            (0.0, 20.0, 0.25, 0.0, 1.0, 0.0, 24.0)
        );
    }

    #[test]
    #[expect(clippy::disallowed_methods, reason = "a plain test waits for mpv on purpose, no runtime is blocked")]
    fn mpv_accepts_the_chain_across_the_slider_range() {
        for amount in [0.0, 0.001, 0.02, 0.05, 0.3, 1.0] {
            let fx = Fx { reverb: amount, echo: amount, bass: 6.0, stereo: 2.0, normalize: true, ..Fx::default() };
            let out = std::process::Command::new("mpv")
                .args(["--no-config", "--no-video", "--ao=null", "--length=0.4", "--really-quiet"])
                .arg(format!("--af={}", chain(&fx, false)))
                .arg("av://lavfi:sine")
                .output()
                .unwrap();
            assert!(out.status.success(), "amount {amount}: {}", String::from_utf8_lossy(&out.stderr));
        }
        for fx in [Fx::default(), Fx { enabled: false, ..Fx::default() }, Fx { bass: 6.0, ..Fx::default() }] {
            let out = std::process::Command::new("mpv")
                .args(["--no-config", "--no-video", "--ao=null", "--length=0.4", "--really-quiet"])
                .arg(format!("--af={}", chain(&fx, true)))
                .arg("av://lavfi:sine")
                .output()
                .unwrap();
            assert!(out.status.success(), "{}: {}", chain(&fx, true), String::from_utf8_lossy(&out.stderr));
        }
    }

    #[test]
    fn measuring_adds_loudnorm_last_and_works_with_effects_off() {
        assert_eq!(chain(&Fx::default(), true), "lavfi=[loudnorm=I=-14:TP=-1.5:LRA=11]");
        assert_eq!(chain(&Fx { enabled: false, bass: 6.0, ..Fx::default() }, true), "lavfi=[loudnorm=I=-14:TP=-1.5:LRA=11]");
        assert_eq!(chain(&Fx { bass: 6.0, ..Fx::default() }, true), "lavfi=[bass=g=6,loudnorm=I=-14:TP=-1.5:LRA=11]");
    }

    #[test]
    fn the_gain_only_ever_lowers_a_track() {
        let from_fixtures = [(0.97999954, -0.97999954), (0.9899998, -0.9899998), (4.25, -4.25), (5.5, -5.5), (-6.4, 0.0), (0.0, 0.0)];
        for (loudness, gain) in from_fixtures {
            assert_eq!(super::gain(loudness), gain, "loudnessDb {loudness}");
        }
    }
}
