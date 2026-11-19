//! Output devices: the list mpv offers, the one the person picked, and following it when it comes and goes.

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

use snellar_core::model::AudioDevice;
use snellar_core::sync::Lock;

use crate::audio::AudioOut;
use crate::outputs;

use super::Shared;

impl Shared {
    pub(super) async fn devices(&self) -> Result<Value> {
        let audio = self.audio().await?;
        let devices = read_devices(&audio.get("audio-device-list").await?);
        let active = audio.get("audio-device").await?;
        let wanted = self.out.wanted.locked().device.clone();
        let listed = outputs::outputs(&devices, &wanted);
        Ok(json!({ "devices": listed.shown, "wanted": listed.target.unwrap_or(wanted), "active": active }))
    }

    pub(super) async fn set_device(&self, name: &str) -> Result<()> {
        let audio = self.audio().await?;
        self.out.wanted.locked().device = name.to_owned();
        let devices = read_devices(&audio.get("audio-device-list").await?);
        self.route(&*audio, &devices).await
    }

    pub(super) async fn route(&self, audio: &dyn AudioOut, devices: &[AudioDevice]) -> Result<()> {
        let wanted = self.out.wanted.locked().device.clone();
        let target = outputs::outputs(devices, &wanted).target.unwrap_or_else(|| "auto".to_owned());
        if audio.get("audio-device").await?.as_str() != Some(target.as_str()) {
            self.out.settled(audio, audio.set("audio-device", json!(target))).await?;
        }
        Ok(())
    }

    pub(super) fn devices_changed(&self, list: &Value) {
        let devices = read_devices(list);
        let Some(this) = self.me.upgrade() else { return };
        self.tasks.spawn("follow the output devices", async move {
            if let Ok(audio) = this.audio().await
                && let Err(e) = this.route(&*audio, &devices).await
            {
                snellar_core::journal::warn("audio", format!("cannot switch the output device: {e:#}"));
            }
        });
    }
}

pub(super) fn read_devices(list: &Value) -> Vec<AudioDevice> {
    Vec::<AudioDevice>::deserialize(list).unwrap_or_default().into_iter().filter(|d| d.name != "auto").collect()
}
