use snellar_core::model::AudioDevice;

const SERVERS: [&str; 2] = ["pipewire/", "pulse/"];
const ALSA_HARDWARE: &str = "alsa/plughw:";

pub struct Outputs {
    pub shown: Vec<AudioDevice>,
    pub target: Option<String>,
}

pub fn outputs(all: &[AudioDevice], wanted: &str) -> Outputs {
    let mut shown = offered(all);
    let target = resolve(all, &shown, wanted);
    if let Some(name) = &target
        && !shown.iter().any(|d| &d.name == name)
        && let Some(device) = all.iter().find(|d| &d.name == name)
    {
        shown.push(device.clone());
    }
    Outputs { shown, target }
}

fn offered(all: &[AudioDevice]) -> Vec<AudioDevice> {
    for server in SERVERS {
        let sinks: Vec<AudioDevice> = all.iter().filter(|d| sink_of(&d.name, server).is_some()).cloned().collect();
        if !sinks.is_empty() {
            return sinks;
        }
    }
    let hardware: Vec<AudioDevice> = all
        .iter()
        .filter(|d| d.name.starts_with(ALSA_HARDWARE))
        .map(|d| AudioDevice { name: d.name.clone(), description: card_of(&d.description).to_owned() })
        .collect();
    if !hardware.is_empty() {
        return hardware;
    }
    all.iter().filter(|d| d.name != "auto" && d.name.contains('/')).cloned().collect()
}

fn resolve(all: &[AudioDevice], shown: &[AudioDevice], wanted: &str) -> Option<String> {
    if wanted == "auto" {
        return None;
    }
    if shown.iter().any(|d| d.name == wanted) {
        return Some(wanted.to_owned());
    }
    let same_sink = SERVERS.iter().find_map(|server| sink_of(wanted, server)).and_then(|sink| {
        shown.iter().find(|d| SERVERS.iter().any(|server| sink_of(&d.name, server) == Some(sink))).map(|d| d.name.clone())
    });
    same_sink.or_else(|| all.iter().any(|d| d.name == wanted).then(|| wanted.to_owned()))
}

fn sink_of<'a>(name: &'a str, server: &str) -> Option<&'a str> {
    name.strip_prefix(server).filter(|sink| !sink.is_empty())
}

fn card_of(description: &str) -> &str {
    description.rsplit_once('/').map_or(description, |(card, _)| card)
}
