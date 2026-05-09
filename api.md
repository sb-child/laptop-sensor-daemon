# Laptop Sensor Daemon D-Bus API 参考文档

`laptop-sensor-daemon` 是一个运行在后台的系统级守护进程，负责整合底层硬件的传感器数据（如物理平板模式开关、重力加速度计、环境光传感器等），并通过标准的 D-Bus 接口将实时状态暴露给上层应用。

## 1. 核心连接信息

所有希望与该服务交互的客户端，请使用以下核心参数建立连接：

- **总线类型 (Bus Type):** `System Bus` (系统总线)
- **服务名称 (Well-known Name):** `org.sbchild.LaptopSensorDaemon`
- **对象路径 (Object Path):** `/org/sbchild/LaptopSensorDaemon`
- **主接口名称 (Interface):** `org.sbchild.LaptopSensorDaemon`

---

## 2. 属性参考 (Properties)

本服务通过主接口暴露以下属性。所有属性均为 **只读 (Read-only)**。当底层硬件传感器发生物理变化时，这些属性的值会自动更新。

| 属性名称      | D-Bus 签名 | 数据类型 | 描述说明                                                                                   | 常见返回值 / 示例                                                                              |
| ------------- | ---------- | -------- | ------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| `TabletMode`  | `b`        | Boolean  | 当前设备是否处于平板模式 (屏幕翻转超过特定角度或键盘被分离)。                              | `true` (平板模式) / `false` (笔记本模式)                                                       |
| `Orientation` | `s`        | String   | 屏幕的物理朝向，通常用于控制操作系统的屏幕自动旋转。如果传感器未就绪，返回 `"undefined"`。 | `"normal"` (正常放置) / `"bottom-up"` (倒置)/ `"left-up"` (向左竖屏) / `"right-up"` (向右竖屏) |
| `Tilt`        | `s`        | String   | 设备的立体倾斜状态，可用于判断设备是立起还是平放。如果传感器未就绪，返回 `"undefined"`。   | `"normal"` (正常立起) / `"tilted-down"` (屏幕面朝下放置) / `"face-up"` (屏幕面朝上平放)        |
| `LightLevel`  | `d`        | Double   | 当前环境光传感器的读数（单位：lux）。如果硬件不支持或未就绪，返回 `0.0`。                  | `401.5`, `0.0`                                                                                 |

---

## 3. 信号与事件订阅 (Signals)

本服务严格遵循 D-Bus 的标准属性接口规范。我们 **没有** 自定义独立的信号方法，而是使用标准的 `PropertiesChanged` 信号。

要订阅传感器数据的实时变化，客户端需要监听以下标准信号：

- **接口:** `org.freedesktop.DBus.Properties`
- **信号名称:** `PropertiesChanged`

**信号负载 (Payload) 解析：**
当收到 `PropertiesChanged` 信号时，它会携带 3 个参数：

1. `String`: 发生变化的接口名称 (固定为 `"org.sbchild.LaptopSensorDaemon"`)。
2. `Dict<String, Variant>`: 发生变化的属性键值对字典（**增量更新**，只包含真正发生变化的属性）。
3. `Array<String>`: 被标记为无效的属性列表（本服务极少使用，通常为空）。

---

## 4. 权限与安全说明

由于本服务运行在系统总线 (`System Bus`) 上，访问受到严格的系统安全策略限制。

- **普通用户/进程访问:** 允许读取所有属性 (`Get` / `GetAll`)，并允许接收属性变化信号 (`PropertiesChanged`)。不需要 `root` 权限即可对接。
- _注意：如果客户端在连接时遭遇 `Access Denied`，请确保系统目录 `/etc/dbus-1/system.d/` 下存在本服务授权配置的 XML 文件，且包含 `<allow receive_sender="org.sbchild.LaptopSensorDaemon"/>` 策略。_

---

## 5. 客户端集成示例

### 示例 A：使用命令行 (Bash / Shell)

**1. 一次性读取所有当前状态 (拉取全量快照):**

```bash
busctl --system introspect org.sbchild.LaptopSensorDaemon /org/sbchild/LaptopSensorDaemon
```

**2. 实时监听传感器变化:**

```bash
dbus-monitor --system "type='signal',sender='org.sbchild.LaptopSensorDaemon'"
```

### 示例 B：使用 Python 3 (`pydbus` 库)

以下脚本展示了如何连接服务、获取初始状态，并挂载事件回调函数持续监听：

```python
import sys
from pydbus import SystemBus
from gi.repository import GLib

# 连接到系统总线并获取代理对象
bus = SystemBus()
try:
    proxy = bus.get("org.sbchild.LaptopSensorDaemon", "/org/sbchild/LaptopSensorDaemon")
except Exception as e:
    print(f"无法连接到服务，请确保守护进程正在运行: {e}")
    sys.exit(1)

# 1. 主动读取全量状态 (Get)
print("=== 当前传感器快照 ===")
print(f"平板模式: {proxy.TabletMode}")
print(f"屏幕方向: {proxy.Orientation}")
print(f"倾斜状态: {proxy.Tilt}")
print(f"环境光照: {proxy.LightLevel} lux")
print("======================\n")

# 2. 定义回调函数，处理增量更新信号
def on_properties_changed(interface_name, changed_properties, invalidated_properties):
    # 过滤掉非本接口的变更信号
    if interface_name != "org.sbchild.LaptopSensorDaemon":
        return

    for prop, new_value in changed_properties.items():
        print(f"[事件流] {prop} 更新为 -> {new_value}")

# 3. 绑定标准属性接口的信号
proxy.PropertiesChanged.connect(on_properties_changed)

# 启动事件循环，保持程序运行并持续监听
print("正在监听硬件实时变更 (按 Ctrl+C 退出)...")
loop = GLib.MainLoop()
try:
    loop.run()
except KeyboardInterrupt:
    print("退出监听。")
```
