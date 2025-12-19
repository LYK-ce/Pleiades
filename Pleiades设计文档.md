# 设计文档

这是一个MVP阶段的运行时框架，主要解决的问题是，为边缘设备集群实现一个统一的运行时框架，使得设备可以：
1. 设备之间自动发现彼此
2. 无需中央服务器，自动协商任务分配
3. 动态适应设备加入/退出
4. 每个设备运行相同代码，角色由运行时动态决定
5. 决策只依赖本地视图，无全局锁



##

## 环境
python 3.13.5
torch 2.9.1+cu128

## 整体架构设计

### 应用层

### 调度层

### 通信层
文件名称 Network.py
位于Src/ 目录下

通信层（Network.py）是边缘设备群的网络通信功能，主要负责以下功能：
1. 定义通信层的总抽线Network
2. 定义服务发现抽象Listener
3. 定义传输服务抽线Transport
4. 定义路由表抽象Routing_Table
5. 通过mDNS实现局域网内设备的发现
6. 维护路由表，加入新的设备或者定期清理离线设备，使每台设备保有局域网内所有其他设备的信息
7. 命令，文件传输，使得每台设备可以向其他设备发送命令或者将模型权重文件、输入等传输给目标设备


#### class Network

外部依赖 zeroconf 用于mDNS服务发现

##### 属性  
- zeroconf            指向zeroconf实例的一个属性
- service_info        zeroconf 服务信息
- listener            服务发现，用于监听其他节点
- routing_Table       路由表，当mDNS发现有新的设备加入或者有设备离开后，更新路由表。这里存储全局加入局域网的所有节点信息。


##### 方法
Start_Network
输入

输出    bool
此函数是通信层的启动方法，它将通过zeroconf配置mDNS，启动mDNS服务。如果所有服务成功启动，那么返回true，如果服务启动失效，返回false

Stop_Network
输入    无
输出    bool
关闭Network的所有网络服务，全部成功关闭返回true，否则返回false

#### class Listener
继承自zeroconf ServiceListener类
用于监听局域网中其他节点的加入或退出

##### 属性  
- routing_Table       路由表，当mDNS发现有新的设备加入或者有设备离开后，更新路由表。这里存储全局加入局域网的所有节点信息。


##### 方法
init
输入
  - routing_Table   路由表
此函数传入路由表，并初始化Listener实例

add_service



#### class Transport
继承自zeroconf ServiceListener类
用于监听局域网中其他节点的加入或退出

##### 属性  
- routing_Table       路由表，当mDNS发现有新的设备加入或者有设备离开后，更新路由表。这里存储全局加入局域网的所有节点信息。


##### 方法
init
输入
  - routing_Table   路由表
此函数传入路由表，并初始化Listener实例

add_service




#### class Routing_Table
路由表类  管理局域网内所有节点的信息
负责
1. 存储节点信息 (ID,IP,端口，设备性能快照)
2. 提供增删改查接口

##### 属性  
- table       路由表


##### 方法
init
初始化一个空路由表

Add_Node
输入
  - node_id
  - ip
  - port
  - property
输出
  - bool
将node id的节点以及对应信息添加到路由表当中。

Remove_Node
输入
  - node_id 要删除的节点id号
输出
  - bool 


### 运行时层
文件名称 Runtime.py
位于 Src/ 目录下

运行时层（Runtime.py）是边缘设备的核心执行引擎，主要负责三大功能：一是通过 Get_System_Snapshot() 方法作为资源探针，实时监测设备的CPU、内存、温度、网络状态等系统资源信息并以字典形式返回；二是通过 Load_Model() 方法加载并管理PyTorch深度学习模型；三是通过 Execute() 方法执行模型推理任务，接收模型输入数据并返回推理结果。运行时层为上层调度层提供了设备资源感知能力和模型执行能力，使得每个边缘设备能够根据自身资源状况动态参与分布式推理任务。

class 名称 Runtime
依赖 Logger.py

#### 属性
model   这个属性用于指明当前要运行的模型，这里是指torch模型

#### 方法

Get_System_Snapshot
输入    无
输出    字典
此函数是资源探针方法，它将获取当前设备的cpu状况，内存，温度，网络状态等等信息，先判断是否存在logger，如果存在调用logger的Log方法，flag为snapshotflag，输入为设备信息。然后通过字典的方式返回。

Load_Model
输入    模型路径
输出    bool
此函数将先先判断是否存在logger，如果存在调用logger的Log方法，flag为Load，输入为prepare loading通过torch加载，将目标路径下的模型加载并赋值给model，然后再次调用Log方法，输入为Loading Success/False，根据Load结果确定，返回是否成功加载

Execute
输入    模型输入
输出    模型输出
此函数先判断model是否已经有模型了，调用logger进行Log，输入为是否存在模型，然后将模型输入送给model进行执行，再次调用logger进行Log，输入为Execution是否成功，如果不成功把错误信息进行输入，并将执行结果返回给调用者。


### 日志组件
文件名称 Logger.py
位于 Src/ 目录下


class 名称 Logger

#### 属性
- log_flag  记录配置，采用二进制的方式进行表示，在记录的时候会比较事件的flag与此flag，按位或操作不为0时表示记录此事件

- log_file  log记录文件，所有log值写入到此文件当中

#### 方法
init
输入    log_flag
输出    无
此函数将首先配置它的log_flag，然后在当前运行目录下创建一个Log目录，然后在此目录下创建一个pleiades_date.log文件，这里的date是当前的日期。

Set_Log_Flag
输入    _log_flag
输出    无
配置新的log_flag

Log
输入    
  - flag    当前事件的flag
  - _input  要记录的一段字符串
输出    
    无

此函数先比较flag与log_flag，通过按位或操作进行比较，只有按位或不等于0时才可以记录。如果可以记录，那么在log_file当中添加新的内容，格式为 date:_input. date为当前事件，_input为要记录的字符串。

#### 全局变量
-   logger 初始值为None，在全局系统启动时再初始化

### 配置组件
文件名称 Config.py
位于 Src/ 目录下


#### 全局常量
以下是日志记录标识符号常量
- EVERYTHING        0xFFFFFFFF
- NOTHING           0x0
  
- RUNTIME_SNAPSHOT  0x1
- RUNTIME_LOAD      0x2
- RUNTIME_EXECUTE   0x4
- RUNTIME           0xF
  
- NETWORK_STATUS    0x10    通信层启动或关闭记录
- NETWORK_INFO      0x20    通信层发送命令、文件记录
- NETWORK_ROUTING   0x40    路由表更新时进行记录
- NETWORK_PING      0x80    通信层进行心跳记录
- NETWORK           0xF0    记录所有通信层消息


以下是通信层的消息类型定义
- PING    0x01    心跳
- COMMAND 0x02    命令
- DATA    0x03    数据块（用于文件传输）

### 消息格式
通信层消息格式：
[4字节长度][1字节消息类型][payload]


version 0.5

