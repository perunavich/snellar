use serde_json::{Value, json};
use snellar_core::model::Fx;

const BANDS: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
const MIN_AMOUNT: f64 = 0.05;
const REVERB_DELAYS: [f64; 4] = [43.0, 71.0, 113.0, 181.0];
const REVERB_DECAYS: [f64; 4] = [0.55, 0.42, 0.32, 0.22];
pub const LIVE: &str = "fx";

fn n(v: f64) -> String {
    format!("{:.2}", v + 0.0).trim_end_matches('0').trim_end_matches('.').to_string()
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
/// The loudness `loudnorm` aims at and YouTube's `loudnessDb` counts from, in LUFS.
const TARGET: f64 = -14.0;
/// The meter in front of the effects while a track is measured: its integrated loudness, and how many samples per
/// channel it took in since mpv last built it, which mpv does for each file and after each seek.
const METER: &str = "ebur128=metadata=1,astats=metadata=1:reset=0:measure_perchannel=none:measure_overall=Number_of_samples";
pub const MEASURED: &str = "af-metadata/measure";
/// ebur128's absolute gate: silence reads as this.
const SILENT: f64 = -70.0;

pub fn gain(loudness_db: f64) -> f64 {
    (-loudness_db).min(0.0)
}

/// A measured integrated loudness on the scale of YouTube's `loudnessDb`.
pub fn loudness_db(lufs: f64) -> f64 {
    lufs - TARGET
}

/// The meter's reading, from mpv's `af-metadata` and the decoded sample rate: the integrated loudness in LUFS and the
/// seconds of the track it covers. Nothing while it has taken in only silence.
pub fn measured(metadata: &Value, rate: &Value) -> Option<(f64, f64)> {
    let number = |key: &str| match &metadata[key] {
        Value::String(s) => s.parse::<f64>().ok(),
        other => other.as_f64(),
    };
    let lufs = number("lavfi.r128.I").filter(|l| l.is_finite() && *l > SILENT)?;
    let rate = rate.as_f64().filter(|r| *r > 0.0)?;
    let seconds = number("lavfi.astats.Overall.Number_of_samples")? / rate;
    Some((lufs, seconds))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Knob {
    pub target: String,
    fixed: String,
    pub option: &'static str,
    pub value: String,
}

fn knobs(fx: &Fx) -> Vec<Knob> {
    let fx = clamp(fx);
    let at = |value: f64, neutral: f64| n(if fx.enabled { value } else { neutral });
    let knob = |target: String, fixed: String, option, value| Knob { target, fixed, option, value };
    let mut all = vec![knob("volume@pre".into(), String::new(), "volume", format!("{}dB", at(fx.preamp, 0.0)))];
    for (i, (freq, gain)) in BANDS.iter().zip(fx.eq).enumerate() {
        all.push(knob(format!("equalizer@b{i}"), format!("f={freq}:t=o:w=1:r=f64:"), "g", at(gain, 0.0)));
    }
    all.push(knob("bass@bass".into(), "r=f64:".into(), "g", at(fx.bass, 0.0)));
    all.push(knob("extrastereo@stereo".into(), "c=0:".into(), "m", at(fx.stereo, 1.0)));
    all
}

fn live(fx: &Fx) -> String {
    let stages: Vec<String> = knobs(fx).iter().map(|k| format!("{}={}{}={}", k.target, k.fixed, k.option, k.value)).collect();
    format!("@{LIVE}:lavfi=[{}]", stages.join(","))
}

fn rebuilt(fx: &Fx, measure: bool) -> Vec<String> {
    let fx = clamp(fx);
    let mut stages = Vec::new();
    if fx.enabled && fx.echo >= MIN_AMOUNT {
        let amount = fx.echo;
        stages.push(format!("@echo:lavfi=[aecho=0.8:0.9:{}:{}]", n(280.0 + 220.0 * amount), n(0.7 * amount)));
    }
    if fx.enabled && fx.reverb >= MIN_AMOUNT {
        let amount = fx.reverb;
        let delays = taps(&REVERB_DELAYS, 1.0);
        let decays = taps(&REVERB_DECAYS, amount);
        stages.push(format!("@reverb:lavfi=[aecho={}:0.88:{delays}:{decays}]", n(1.0 - 0.25 * amount)));
    }
    if fx.enabled && fx.normalize {
        stages.push("@normalize:lavfi=[dynaudnorm]".into());
    }
    if measure {
        stages.push(format!("@loudness:lavfi=[{LOUDNORM}]"));
    }
    stages
}

fn af(written: &Fx, fx: &Fx, measure: bool) -> String {
    let mut wrappers = Vec::new();
    if measure {
        wrappers.push(format!("@measure:lavfi=[{METER}]"));
    }
    wrappers.push(live(written));
    wrappers.extend(rebuilt(fx, measure));
    wrappers.join(",")
}

fn properties(fx: &Fx) -> [(&'static str, Value); 2] {
    let fx = clamp(fx);
    [("audio-pitch-correction", json!(fx.pitch_lock)), ("speed", json!(if fx.enabled { fx.speed } else { 1.0 }))]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    written: Fx,
    playing: Fx,
    measure: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub af: Option<String>,
    pub commands: Vec<Knob>,
    pub properties: Vec<(&'static str, Value)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Live,
    Settle,
    Reassert,
}

pub fn plan(now: Option<&Applied>, want: &Fx, measure: bool, mode: Mode) -> (Plan, Applied) {
    let want = clamp(want);
    let rebuild = now.is_none_or(|now| {
        rebuilt(&now.playing, now.measure) != rebuilt(&want, measure) || (mode == Mode::Settle && knobs(&now.written) != knobs(&want))
    });
    let written = match now {
        Some(now) if !rebuild => now.written.clone(),
        _ => want.clone(),
    };
    let next = af(&written, &want, measure);
    let af = now.is_none_or(|now| af(&now.written, &now.playing, now.measure) != next).then_some(next);
    let commands = match now {
        Some(now) if af.is_none() || knobs(&now.written) == knobs(&written) => {
            let (held, playing) = (knobs(&now.written), knobs(&now.playing));
            let differs = |i: usize, value: &str| playing[i].value != value || (mode == Mode::Reassert && held[i].value != value);
            knobs(&want).into_iter().enumerate().filter(|(i, k)| differs(*i, &k.value)).map(|(_, k)| k).collect()
        }
        _ => Vec::new(),
    };
    let before = now.map(|now| properties(&now.playing));
    let properties =
        properties(&want).into_iter().enumerate().filter(|(i, p)| before.as_ref().is_none_or(|b| b[*i] != *p)).map(|(_, p)| p).collect();
    (Plan { af, commands, properties }, Applied { written, playing: want, measure })
}
