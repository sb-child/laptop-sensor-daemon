# gpu-hacks

## 屏幕旋转180度之后画面撕裂

这是显卡驱动没适配好导致的，可以~~暂时~~永久关闭 FBC(Frame Buffer Compression)。

```bash
sudo grubby --update-kernel=/vmlinuz-你的内核名字 --args="xe.enable_fbc=0"
```
