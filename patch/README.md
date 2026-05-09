# patch

我对内核模块做的一点点魔改

## [hp-wmi](hp-wmi-patched/)

添加了惠普 Flip 笔记本的 360 度铰链支持。

这样它能在**开机第一次旋转屏轴**时突然多出一个 input device 给你汇报接下来的 `SW_TABLET_MODE` 事件。

```bash
make -C /usr/src/kernels/你的内核名字 M=$(pwd) modules

xz -z --check=crc32 --lzma2=dict=1MiB hp-wmi.ko

sudo cp hp-wmi.ko.xz /lib/modules/你的内核名字/kernel/drivers/platform/x86/hp/
```
