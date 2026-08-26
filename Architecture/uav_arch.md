# UAV 架构说明（uav_arch）

> 创建日期：2026-08-21
> 范围：DRF450（F450-4B）无人机平台的树莓派环境配置与飞控接入
> 平台技术规格详见 `docs/design_doc/UAV.md`
> 本文记录实际部署中确认的硬件连接、网络配置与踩坑记录

---

## 1. 概述

无人机平台采用 **树莓派 4B（机载计算机）+ Pixhawk 2.4.8（飞控）** 分离架构。树莓派通过 **UART 串口 + MAVLink 协议** 与飞控通信，通过 **WiFi / libp2p** 与地面站及他机协同。

```
地面站(Pictor/电脑) ──WiFi/libp2p──▶ 树莓派4B ──串口/MAVLink──▶ Pixhawk 飞控 ──▶ 电机/桨
```

## 2. 硬件连接（树莓派 ↔ 飞控）

| 项目 | 说明 |
|---|---|
| 连接方式 | UART 串口直连飞控 TELEM 口 |
| 树莓派引脚 | 物理 Pin8 = TXD（GPIO14）、Pin10 = RXD（GPIO15）、Pin14 = GND |
| 接线 | 飞控 TX → 树莓派 RX(Pin10)；飞控 RX → 树莓派 TX(Pin8)；GND 共地 |
| 串口设备 | `/dev/serial0` → `/dev/ttyAMA0`（主 UART PL011，蓝牙已让出） |
| 波特率 | 921600（备用 57600） |
| 电平 | 3.3V TTL，飞控 TELEM 同为 3.3V，电压匹配 |

**串口软链**（已确认）：
```
/dev/serial0 -> ttyAMA0   # 主 UART，稳定，用于飞控
/dev/serial1 -> ttyS0     # mini UART，未用
```

**软件协议层**：MAVLink（不自己写串口字节协议）
- Rust 侧：`mavlink` crate（`ardupilotmega` 方言，等价于 pymavlink）
- Python 侧（树莓派已装）：pymavlink 2.4.49、MAVProxy、pyserial 3.5

**串口权限**：设备属组 `root:dialout`，运行程序/脚本的用户需在 `dialout` 组。

## 3. 树莓派网络配置

### 3.1 网络管理

- NetworkManager 管理网络（netplan `renderer: NetworkManager`）。
- netplan 仅配 `eth0` 有线静态 IP（`192.168.1.123/24`），`wlan0` 完全由 NetworkManager 管理。
- 目标形态：树莓派与地面站连接**同一个 WiFi**，供 libp2p/mDNS 发现协同。

### 3.2 ⚠️ 开机自启动热点（踩坑记录，2026-08-21）

**现象**：树莓派重启后，手动连接的 WiFi 被顶掉，自动切回热点模式（SSID `DronePi`）。即使把 NetworkManager 里 `Hotspot` 连接的 `autoconnect` 设为 `no` 也无效。

**根因**：root 的 cron 里有一条开机自启动任务，**开机 10 秒后强制拉起热点**：

```cron
@reboot sleep 10 && nmcli connection up Hotspot
```

它用 `nmcli connection up` 显式激活连接，**绕过了 NetworkManager 的 autoconnect 设置**，所以 GUI/`nmcli modify autoconnect no` 都治不了。

**排查方法**（关键：用日志定位是谁拉起的）：

```bash
sudo journalctl -b | grep -iE "hotspot|nmcli dev wifi|create_ap"
```

日志中会看到：

```
Aug 21 15:01:35 CRON[1205]: (root) CMD (sleep 10 && nmcli connection up Hotspot)
```

`CRON[...] (root) CMD (...)` 即为线索 → 检查 `sudo crontab -l`。

**解决方案（注释掉，保留可恢复）**：

```bash
# 注释掉含 Hotspot 的 cron 行（行首加 #）
sudo crontab -l | sed '/Hotspot/ s|^|#|' | sudo crontab -
```

验证：

```bash
sudo crontab -l   # 应看到 #@reboot sleep 10 && nmcli connection up Hotspot
sudo reboot
nmcli connection show --active   # 重启后 wlan0 应连自己的 WiFi，热点不再自起
```

**恢复热点**（需要时去掉 `#`）：

```bash
sudo crontab -l | sed '/Hotspot/ s|^#||' | sudo crontab -
```

### 3.3 连接自己的 WiFi（持久化）

```bash
nmcli device wifi connect "SSID" password "密码"
```

该命令自动保存连接并设为开机自动连接。

**注意**：历史保存的杂项 WiFi 若都是 `autoconnect yes`，开机可能连错网。建议只保留自己的 WiFi 自动连接，其余 `autoconnect no` 或删除：

```bash
nmcli -f NAME,AUTOCONNECT connection show            # 查看
sudo nmcli connection modify "名字" connection.autoconnect no   # 关闭某条
sudo nmcli connection delete "名字"                   # 删除某条
```

## 4. 关键账号 / IP

| 项目 | 值 |
|---|---|
| 树莓派用户 | `ubuntu`（另有 `drobotics` 用户） |
| 热点模式默认 IP | `10.42.0.20`（连自己 WiFi 后改由路由器 DHCP 分配） |
| 热点默认密码 | `123456abcd` |
| SSH 密码 | `123456abc` |
| 飞控地面站 | Mission Planner / QGroundControl |
| 飞控固件 | ArduCopter V4.5.7 |

## 5. 验证命令速查

```bash
# 串口设备/软链
ls -l /dev/serial*

# 打开串口（飞控不用上电）
python3 -c "import serial; s=serial.Serial('/dev/ttyAMA0', 921600, timeout=1); print('打开成功:', s.name); s.close()"

# MAVProxy 收心跳（飞控需上电）
mavproxy.py --master=/dev/ttyAMA0 --baudrate=921600
# 成功标志：出现 MAV> 提示符 + 日志滚动 heartbeat
```

## 附：本次环境排查排除清单

排查开机热点时依次排除（均非根因）：
- `rc.local`：仅 `uhubctl` 重启 USB 集线器，无关热点
- `/etc/NetworkManager/dispatcher.d/`：仅系统默认脚本
- netplan（`/etc/netplan/*.yaml`）：仅 `eth0` 静态 IP，无 `wlan0` AP 配置
- **根因在 cron**（`sudo crontab -l`）
