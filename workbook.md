# Workbook

## Task1: 实现运行时层
开始: 2025-12-16T19:15:00+08:00
结束: 2025-12-16T19:15:52+08:00
状态: 完成
依赖: 无
实现: Src/Runtime.py
内容: Runtime类,含model属性,Get_System_Snapshot/Load_Model/Execute方法
技术: torch,psutil,支持GPU,包含系统资源监控(CPU/内存/温度/网络/GPU)

## Task2: 完成Test目录下的所有测试相关任务(含简化修正)
开始: 2025-12-16T19:26:38+08:00
结束: 2025-12-16T19:44:06+08:00
状态: 完成
依赖: Task1
实现文件:
- Test/Model/CNN_Model.py: CNN模型(3层conv+bn+pool,fc分类头,参数1.1M,使用torch.jit.trace保存)
- Test/Model/ViT_Model.py: ViT模型(patch_embed+6层transformer+分类头,参数2.7M,使用torch.jit.trace保存)
- Test/Runtime_Test.py: 简化版,3个测试函数,使用assert验证,无unicode问题,Test_Get_System_Snapshot包含网络信息输出
- Test/Test.py: 统一测试入口,支持命令行参数(runtime/models/all/list)
- Src/Runtime.py: 更新Load_Model支持torch.jit模型加载
修正: 使用torch.jit.trace避免类定义依赖,修复unicode输出问题,所有测试通过,增加网络接口数量/发送接收字节输出
技术: torch.nn,torch.jit,argparse,简单assert测试

## Task3: 增强网络信息并添加测试文档
开始: 2025-12-16T19:46:39+08:00
结束: 2025-12-16T19:48:29+08:00
状态: 完成
依赖: Task2
实现内容:
- Src/Runtime.py: Get_System_Snapshot增加带宽计算(bytes_sent_per_sec/bytes_recv_per_sec/bandwidth_mbps),网络延迟测量(network_latency_ms)
- Test/Runtime_Test.py: Test_Get_System_Snapshot增加带宽和延迟输出
- Test/test.md: 完整的Test目录说明文档(目录结构/使用方法/测试内容/模型说明/故障排除)
技术细节: 双次采样计算实时带宽,socket测试延迟,完善文档
测试结果: 所有测试通过,带宽延迟正常显示

## Task4: 完成配置组件、日志组件并更新Runtime组件
开始: 2025-12-18T13:33:32+08:00
结束: 2025-12-18T13:35:14+08:00
状态: 完成
依赖: Task3
实现文件:
- Src/Config.py: 配置组件,定义日志标识符常量(RUNTIME_SNAPSHOT=0b001, RUNTIME_LOAD=0b010, RUNTIME_EXECUTE=0b100, RUNTIME=0b111),使用typing.Final确保常量不可变
- Src/Logger.py: 日志组件,Logger类包含log_flag属性(二进制标志位控制),log_file属性,init方法创建Log目录和日志文件(pleiades_YYYYMMDD.log格式),Set_Log_Flag方法更新标志位,Log方法通过按位与操作判断是否记录日志,全局logger变量初始为None
- Src/Runtime.py: 更新三个方法集成Logger,Get_System_Snapshot在开始和结束时记录日志(flag=RUNTIME_SNAPSHOT),Load_Model在准备加载/加载成功/加载失败时记录日志(flag=RUNTIME_LOAD),Execute在检查模型/开始执行/执行成功/执行失败时记录日志(flag=RUNTIME_EXECUTE),所有日志操作都先判断logger是否为None
技术细节: 使用按位与(&)操作判断日志记录条件,日志格式为"时间戳: 消息内容",支持UTF-8编码,日志目录自动创建,文件名包含日期便于管理
