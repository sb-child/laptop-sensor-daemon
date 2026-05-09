use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};
use zbus::zvariant::{OwnedValue, Value};

#[derive(Debug)]
pub enum SystemEvent {
    TabletMode(bool),
    SensorProperty { name: String, value: OwnedValue },
}

#[derive(Clone)]
pub struct EventBus {
    pub state_rx: watch::Receiver<DeviceState>,
    pub event_tx: broadcast::Sender<DeviceEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceState {
    pub tablet_mode: Option<bool>,
    pub orientation: Option<String>,
    pub tilt: Option<String>,
    pub light_level: Option<f64>,
    pub proximity_near: Option<bool>,
}

impl DeviceState {
    pub fn apply_event(&mut self, event: &DeviceEvent) {
        if let Some(v) = event.tablet_mode {
            self.tablet_mode = Some(v);
        }
        if let Some(ref v) = event.orientation {
            self.orientation = Some(v.clone());
        }
        if let Some(ref v) = event.tilt {
            self.tilt = Some(v.clone());
        }
        if let Some(v) = event.light_level {
            self.light_level = Some(v);
        }
        if let Some(v) = event.proximity_near {
            self.proximity_near = Some(v);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tablet_mode: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tilt: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_level: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub proximity_near: Option<bool>,
}

pub fn parse_system_event(sys_event: SystemEvent) -> Option<DeviceEvent> {
    let mut event = DeviceEvent::default();
    let mut has_changes = false;

    match sys_event {
        SystemEvent::TabletMode(is_tablet) => {
            event.tablet_mode = Some(is_tablet);
            has_changes = true;
        }
        SystemEvent::SensorProperty { name, value } => {
            let dbus_value = &*value;
            match name.as_str() {
                "AccelerometerOrientation" => {
                    if let Ok(s) = String::try_from(dbus_value) {
                        event.orientation = Some(s);
                        has_changes = true;
                    }
                }
                "AccelerometerTilt" => {
                    if let Ok(s) = String::try_from(dbus_value) {
                        event.tilt = Some(s);
                        has_changes = true;
                    }
                }
                "LightLevel" => {
                    if let Ok(v) = f64::try_from(dbus_value) {
                        event.light_level = Some(v);
                        has_changes = true;
                    }
                }
                "ProximityNear" => {
                    if let Ok(v) = bool::try_from(dbus_value) {
                        event.proximity_near = Some(v);
                        has_changes = true;
                    }
                }
                _ => {}
            }
        }
    }

    if has_changes { Some(event) } else { None }
}

pub fn apply_dbus_value_to_event(key: &str, value: &Value<'_>, event: &mut DeviceEvent) {
    match key {
        "AccelerometerOrientation" => {
            if let Ok(s) = String::try_from(value) {
                event.orientation = Some(s);
            }
        }
        "AccelerometerTilt" => {
            if let Ok(s) = String::try_from(value) {
                event.tilt = Some(s);
            }
        }
        "LightLevel" => {
            if let Ok(v) = f64::try_from(value) {
                event.light_level = Some(v);
            }
        }
        "ProximityNear" => {
            if let Ok(v) = bool::try_from(value) {
                event.proximity_near = Some(v);
            }
        }
        _ => {}
    }
}
