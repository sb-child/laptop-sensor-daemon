# laptop-sensor-daemon

你知道吗，还没有几个 Linux 窗管适配过屏幕自动旋转... 甚至不支持"平板模式"。所以我给内核模块打了[补丁](./patch/README.md)，顺便写了这个项目。

只是为了让[小琳琳](https://github.com/AkiharaHoshina)能用最新最热的 HP OmniBook X Flip Laptop 14 办公和[打音游](https://github.com/AkiharaHoshina/prpr-miniquadWayland/tree/patch-wayland)。

## 安装

```bash
cargo b -r

sudo cp target/release/laptop-sensor-daemon /usr/local/bin/laptop-sensor-daemon

sudo cp laptop-sensor-daemon.service /etc/systemd/system/laptop-sensor-daemon.service

sudo cp org.sbchild.LaptopSensorDaemon.conf /etc/dbus-1/system.d/org.sbchild.LaptopSensorDaemon.conf

sudo systemctl daemon-reload

sudo systemctl enable --now laptop-sensor-daemon
```

## 对接

看[这里](./api.md)

## 客户端示例

- [caonima](https://github.com/sb-child/caonima)
