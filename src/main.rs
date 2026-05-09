use evdev::{Device, EventType, SwitchCode};
use futures_util::StreamExt;
use snafu::{ResultExt, Snafu};
use std::os::unix::fs::FileTypeExt;
use tokio::signal;
use tokio::sync::broadcast;
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

#[derive(Debug, Clone)]
pub enum SystemEvent {
    TabletMode(bool),
    SensorProperty { name: String, value: String },
}

fn find_tablet_mode_device() -> Option<Device> {
    let mut evdev_dir = std::fs::read_dir("/dev/input").ok()?;
    while let Some(Ok(entry)) = evdev_dir.next() {
        let path = entry.path();
        if let Ok(metadata) = std::fs::metadata(&path) {
            if !metadata.file_type().is_char_device() {
                continue;
            }
        }
        if let Ok(device) = Device::open(&path) {
            if device
                .supported_switches()
                .map_or(false, |sw| sw.contains(SwitchCode::SW_TABLET_MODE))
            {
                return Some(device);
            }
        }
    }
    None
}

async fn spawn_evdev_listener(event_tx: broadcast::Sender<SystemEvent>) {
    tokio::spawn(async move {
        let Some(device) = find_tablet_mode_device() else {
            warn!("未在系统中检测到平板模式物理开关。");
            return;
        };

        info!(
            "找到平板模式开关: {:?}",
            device.physical_path().unwrap_or("Unknown")
        );

        if let Ok(state) = device.get_switch_state() {
            let is_tablet = state.contains(SwitchCode::SW_TABLET_MODE);
            info!(
                "初始物理开关状态: {}",
                if is_tablet {
                    "Tablet (平板模式)"
                } else {
                    "Laptop (笔记本模式)"
                }
            );
            let _ = event_tx.send(SystemEvent::TabletMode(is_tablet));
        }

        let mut stream = match device.into_event_stream() {
            Ok(s) => s,
            Err(e) => {
                error!("无法创建 evdev 异步流: {}", e);
                return;
            }
        };

        while let Ok(event) = stream.next_event().await {
            if event.event_type() == EventType::SWITCH
                && event.code() == SwitchCode::SW_TABLET_MODE.0
            {
                let is_tablet = event.value() == 1;
                let _ = event_tx.send(SystemEvent::TabletMode(is_tablet));
            }
        }
    });
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
    event_tx: broadcast::Sender<SystemEvent>,
) -> Result<SensorProxyProxy<'static>> {
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

    info!("正在唤醒可用传感器...");
    if has_accelerometer {
        proxy
            .claim_accelerometer()
            .await
            .context(SensorMethodCallSnafu)?;
        info!("加速度计 (已唤醒)");
    }
    if has_ambient_light {
        proxy.claim_light().await.context(SensorMethodCallSnafu)?;
        info!("环境光传感器 (已唤醒)");
    }
    if has_proximity {
        proxy
            .claim_proximity()
            .await
            .context(SensorMethodCallSnafu)?;
        info!("接近光传感器 (已唤醒)");
    }

    let tx_clone = event_tx.clone();
    let props_proxy_clone = props_proxy.clone();

    // 异步获取全量状态，延迟以确保传感器已完全唤醒
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(props) = props_proxy_clone.get_all(interface_name).await {
            debug!("=== 当前传感器全量状态 ===");
            for (key, value) in props {
                let val_str = format!("{:?}", value);
                debug!("{}: {}", key, val_str);
                let _ = tx_clone.send(SystemEvent::SensorProperty {
                    name: key.to_string(),
                    value: val_str,
                });
            }
            debug!("=========================");
        }
    });

    let mut changes_stream = props_proxy
        .receive_properties_changed()
        .await
        .context(PropertyBuildSnafu)?;

    // 监听属性变化事件
    tokio::spawn(async move {
        while let Some(signal) = changes_stream.next().await {
            if let Ok(args) = signal.args() {
                for (key, value) in args.changed_properties() {
                    let val_str = format!("{:?}", value);
                    let _ = event_tx.send(SystemEvent::SensorProperty {
                        name: key.to_string(),
                        value: val_str,
                    });
                }
            }
        }
    });

    Ok(proxy)
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化 tracing 格式化输出
    tracing_subscriber::fmt::init();

    let (event_tx, mut event_rx) = broadcast::channel::<SystemEvent>(100);

    spawn_evdev_listener(event_tx.clone()).await;

    let conn = Connection::system().await.context(BusConnectionSnafu)?;
    let sensor_proxy = setup_sensor_proxy(&conn, event_tx.clone()).await?;

    info!("正在持续监听事件流 (按 Ctrl+C 退出)...");

    loop {
        tokio::select! {
            Ok(event) = event_rx.recv() => {
                match event {
                    SystemEvent::TabletMode(is_tablet) => {
                        let state = if is_tablet { "Tablet Mode (平板模式)" } else { "Laptop Mode (笔记本模式)" };
                        info!("[Hardware Switch] 状态变更: {}", state);
                    }
                    SystemEvent::SensorProperty { name, value } => {
                        info!("[Sensor Event] {} 变更为: {}", name, value);
                    }
                }
            }
            _ = signal::ctrl_c() => {
                info!("收到中断信号，正在安全清理资源...");
                break;
            }
        }
    }

    // 优雅释放传感器占用
    let _ = sensor_proxy.release_light().await;
    let _ = sensor_proxy.release_accelerometer().await;
    let _ = sensor_proxy.release_proximity().await;

    info!("退出完成。");
    Ok(())
}
