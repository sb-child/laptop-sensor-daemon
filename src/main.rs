use evdev::{Device, EventType, SwitchCode};
use futures_util::StreamExt;
use laptop_sensor_daemon::event::{
    DeviceEvent, DeviceState, EventBus, SystemEvent, parse_system_event,
};
use laptop_sensor_daemon::service::start_dbus_server;
use snafu::{ResultExt, Snafu};
use std::os::unix::fs::FileTypeExt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{broadcast, watch};
use tokio::{signal, sync::mpsc};
use tracing::{debug, error, info, warn};
use zbus::{Connection, fdo::PropertiesProxy, names::InterfaceName};

#[derive(Debug, Snafu)]
pub enum DaemonError {
    #[snafu(display("D-Bus 系统总线连接失败: {source}"))]
    BusConnection { source: zbus::Error },

    #[snafu(display("创建 SensorProxy 代理对象失败: {source}"))]
    ProxyCreation { source: zbus::Error },

    #[snafu(display("获取 D-Bus 属性失败: {source}"))]
    PropertyAccess { source: zbus::fdo::Error },

    #[snafu(display("构建属性代理失败: {source}"))]
    PropertyBuild { source: zbus::Error },

    #[snafu(display("接口名称转换失败: {source}"))]
    InterfaceNameConversion { source: zbus::names::Error },

    #[snafu(display("调用传感器唤醒/释放方法失败: {source}"))]
    SensorMethodCall { source: zbus::Error },
}

type Result<T, E = DaemonError> = std::result::Result<T, E>;

fn find_tablet_mode_devices() -> Vec<Device> {
    let mut devices = Vec::new();
    let Ok(evdev_dir) = std::fs::read_dir("/dev/input") else {
        return devices;
    };
    for entry in evdev_dir.flatten() {
        let path = entry.path();
        if !path.is_dir()
            && path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with("event")
        {
            if let Ok(device) = Device::open(&path) {
                if device
                    .supported_switches()
                    .map_or(false, |sw| sw.contains(SwitchCode::SW_TABLET_MODE))
                {
                    devices.push(device);
                }
            }
        }
    }
    devices
}

async fn spawn_evdev_listener(event_tx: mpsc::Sender<SystemEvent>) {
    let devices = find_tablet_mode_devices();
    if devices.is_empty() {
        warn!("未在系统中检测到任何声明支持平板模式的物理开关。");
        return;
    }
    for device in devices {
        let tx_clone = event_tx.clone();
        tokio::spawn(async move {
            let phys = device.physical_path().unwrap_or("Unknown").to_string();
            let name = device.name().unwrap_or("Unknown").to_string();
            info!("挂载平板模式开关监听: {} (物理路径: {})", name, phys);
            if let Ok(state) = device.get_switch_state() {
                let is_tablet = state.contains(SwitchCode::SW_TABLET_MODE);
                let _ = tx_clone.send(SystemEvent::TabletMode(is_tablet)).await;
            }
            let mut stream = match device.into_event_stream() {
                Ok(s) => s,
                Err(e) => {
                    error!("无法创建 evdev 异步流 [{}]: {}", phys, e);
                    return;
                }
            };
            while let Ok(event) = stream.next_event().await {
                if event.event_type() == EventType::SWITCH
                    && event.code() == SwitchCode::SW_TABLET_MODE.0
                {
                    let is_tablet = event.value() == 1;
                    let _ = tx_clone.send(SystemEvent::TabletMode(is_tablet)).await;
                }
            }
        });
    }
}

#[zbus::proxy(
    interface = "net.hadess.SensorProxy",
    default_service = "net.hadess.SensorProxy",
    default_path = "/net/hadess/SensorProxy"
)]
trait SensorProxy {
    fn claim_light(&self) -> zbus::Result<()>;
    fn claim_accelerometer(&self) -> zbus::Result<()>;
    fn claim_proximity(&self) -> zbus::Result<()>;

    fn release_light(&self) -> zbus::Result<()>;
    fn release_accelerometer(&self) -> zbus::Result<()>;
    fn release_proximity(&self) -> zbus::Result<()>;
}

async fn setup_sensor_proxy(
    connection: &Connection,
    event_tx: mpsc::Sender<SystemEvent>,
) -> Result<(
    SensorProxyProxy<'static>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
)> {
    let proxy = SensorProxyProxy::new(connection)
        .await
        .context(ProxyCreationSnafu)?;
    let props_proxy = PropertiesProxy::builder(connection)
        .destination("net.hadess.SensorProxy")
        .context(PropertyBuildSnafu)?
        .path("/net/hadess/SensorProxy")
        .context(PropertyBuildSnafu)?
        .build()
        .await
        .context(PropertyBuildSnafu)?;
    let interface_name =
        InterfaceName::try_from("net.hadess.SensorProxy").context(InterfaceNameConversionSnafu)?;
    let initial_props = props_proxy
        .get_all(interface_name.clone())
        .await
        .context(PropertyAccessSnafu)?;
    let has_accelerometer = initial_props
        .get("HasAccelerometer")
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false);
    let has_ambient_light = initial_props
        .get("HasAmbientLight")
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false);
    let has_proximity = initial_props
        .get("HasProximity")
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false);
    let mut changes_stream = props_proxy
        .receive_properties_changed()
        .await
        .context(PropertyBuildSnafu)?;
    let event_tx_clone = event_tx.clone();
    tokio::spawn(async move {
        while let Some(signal) = changes_stream.next().await {
            debug!("new signal");
            if let Ok(args) = signal.args() {
                for (key, value) in args.changed_properties() {
                    debug!("changed_properties: key {}, value {:?}", key, value);
                    let Ok(value_owned) = value.try_to_owned() else {
                        continue;
                    };
                    let _ = event_tx_clone
                        .send(SystemEvent::SensorProperty {
                            name: key.to_string(),
                            value: value_owned,
                        })
                        .await;
                }
            }
        }
    });
    let tx_clone_initial = event_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(props) = props_proxy.get_all(interface_name).await {
            debug!("get_all: new initial props");
            for (key, value) in props {
                debug!("key {}, value {:?}", key, value);
                let _ = tx_clone_initial
                    .send(SystemEvent::SensorProperty {
                        name: key.to_string(),
                        value,
                    })
                    .await;
            }
        }
    });
    let claimed_accel = Arc::new(AtomicBool::new(false));
    let claimed_light = Arc::new(AtomicBool::new(false));
    let claimed_prox = Arc::new(AtomicBool::new(false));
    info!("已发送传感器并发唤醒指令...");
    if has_accelerometer {
        let proxy_clone = proxy.clone();
        let flag = claimed_accel.clone();
        tokio::spawn(async move {
            if proxy_clone.claim_accelerometer().await.is_ok() {
                flag.store(true, Ordering::SeqCst);
                info!("加速度计 (就绪)");
            } else {
                warn!("加速度计唤醒失败");
            }
        });
    }
    if has_ambient_light {
        let proxy_clone = proxy.clone();
        let flag = claimed_light.clone();
        tokio::spawn(async move {
            if proxy_clone.claim_light().await.is_ok() {
                flag.store(true, Ordering::SeqCst);
                info!("环境光传感器 (就绪)");
            } else {
                warn!("环境光传感器唤醒失败");
            }
        });
    }
    if has_proximity {
        let proxy_clone = proxy.clone();
        let flag = claimed_prox.clone();
        tokio::spawn(async move {
            if proxy_clone.claim_proximity().await.is_ok() {
                flag.store(true, Ordering::SeqCst);
                info!("接近光传感器 (就绪)");
            } else {
                warn!("接近光传感器唤醒失败");
            }
        });
    }
    Ok((proxy, claimed_accel, claimed_light, claimed_prox))
}

pub fn spawn_event_processor(mut internal_rx: mpsc::Receiver<SystemEvent>) -> EventBus {
    let (state_tx, state_rx) = watch::channel(DeviceState::default());
    let (event_tx, _event_rx) = broadcast::channel::<DeviceEvent>(100);
    let bus_event_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut current_state = DeviceState::default();
        while let Some(sys_event) = internal_rx.recv().await {
            debug!("new sys_event");
            if let Some(device_event) = parse_system_event(sys_event) {
                current_state.apply_event(&device_event);
                let _ = state_tx.send(current_state.clone());
                if bus_event_tx.receiver_count() > 0 {
                    let _ = bus_event_tx.send(device_event.clone());
                }
                if let Ok(json) = serde_json::to_string(&device_event) {
                    debug!("status {}", json);
                }
            }
        }
    });
    EventBus { state_rx, event_tx }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let (internal_tx, internal_rx) = mpsc::channel::<SystemEvent>(100);
    let event_bus = spawn_event_processor(internal_rx);
    spawn_evdev_listener(internal_tx.clone()).await;
    let (conn, dbus_server_task) = start_dbus_server(event_bus)
        .await
        .context(BusConnectionSnafu)?;
    let (sensor_proxy, claimed_accel, claimed_light, claimed_prox) =
        setup_sensor_proxy(&conn, internal_tx.clone()).await?;
    info!("后台服务已就绪，正在持续监听硬件...");

    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("收到中断信号，正在安全清理资源...");
        }
        // ...
    }

    dbus_server_task.abort();
    // 最好的清理就是不清理
    if claimed_light.load(Ordering::SeqCst) {
        let _ = sensor_proxy.release_light().await;
    }
    if claimed_accel.load(Ordering::SeqCst) {
        let _ = sensor_proxy.release_accelerometer().await;
    }
    if claimed_prox.load(Ordering::SeqCst) {
        let _ = sensor_proxy.release_proximity().await;
    }

    info!("退出完成。");
    Ok(())
}
