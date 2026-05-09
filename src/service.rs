use std::sync::Arc;
use tokio::{sync::RwLock, task::JoinHandle};
use zbus::{Connection, connection::Builder, interface, object_server::InterfaceRef};

use crate::event::{DeviceEvent, DeviceState, EventBus};
pub struct SensorService {
    state: Arc<RwLock<DeviceState>>,
}

#[interface(name = "org.sbchild.LaptopSensorDaemon")]
impl SensorService {
    #[zbus(property)]
    async fn tablet_mode(&self) -> bool {
        self.state.read().await.tablet_mode.unwrap_or(false)
    }

    #[zbus(property)]
    async fn orientation(&self) -> String {
        self.state
            .read()
            .await
            .orientation
            .clone()
            .unwrap_or_else(|| "undefined".to_string())
    }

    #[zbus(property)]
    async fn tilt(&self) -> String {
        self.state
            .read()
            .await
            .tilt
            .clone()
            .unwrap_or_else(|| "undefined".to_string())
    }

    #[zbus(property)]
    async fn light_level(&self) -> f64 {
        self.state.read().await.light_level.unwrap_or(0.0)
    }
}

pub async fn setup_dbus_service(
    conn: &Connection,
    initial_state: Arc<RwLock<DeviceState>>,
) -> zbus::Result<InterfaceRef<SensorService>> {
    let service = SensorService {
        state: initial_state,
    };
    let path = "/org/sbchild/LaptopSensorDaemon";

    conn.object_server().at(path, service).await?;
    conn.object_server()
        .interface::<_, SensorService>(path)
        .await
}

pub async fn notify_and_update(
    iface_ref: &InterfaceRef<SensorService>,
    event: DeviceEvent,
) -> zbus::Result<()> {
    {
        let iface = iface_ref.get().await;
        let mut state = iface.state.write().await;
        state.apply_event(&event);
    }
    let ctxt = iface_ref.signal_emitter();
    if event.tablet_mode.is_some() {
        iface_ref.get().await.tablet_mode_changed(ctxt).await?;
    }
    if event.orientation.is_some() {
        iface_ref.get().await.orientation_changed(ctxt).await?;
    }
    if event.tilt.is_some() {
        iface_ref.get().await.tilt_changed(ctxt).await?;
    }
    if event.light_level.is_some() {
        iface_ref.get().await.light_level_changed(ctxt).await?;
    }
    Ok(())
}

pub async fn start_dbus_server(
    conn: &Connection,
    event_bus: EventBus,
) -> zbus::Result<JoinHandle<()>> {
    let initial_state = event_bus.state_rx.borrow().clone();
    let global_state = Arc::new(RwLock::new(initial_state));
    let iface_handle = setup_dbus_service(conn, global_state).await?;
    let mut rx = event_bus.event_tx.subscribe();
    let h = tokio::spawn(async move {
        while let Ok(device_event) = rx.recv().await {
            if let Err(e) = notify_and_update(&iface_handle, device_event).await {
                tracing::error!("D-Bus 信号发送失败: {}", e);
            }
        }
    });
    Ok(h)
}
