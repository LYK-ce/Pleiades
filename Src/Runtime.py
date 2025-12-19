#Presented by KeJi
#Date : 2025-12-18

import torch
import psutil
import socket
import time
import platform
from Src.Logger import logger
from Src.Config import RUNTIME_SNAPSHOT, RUNTIME_LOAD, RUNTIME_EXECUTE

class Runtime:
    """
    运行时框架核心类
    负责模型加载、执行和系统资源监控
    """
    
    def __init__(self):
        """初始化Runtime实例"""
        self.model = None
    
    def Get_System_Snapshot(self):
        """
        获取当前设备的系统快照信息
        
        输入: 无
        输出: dict - 包含CPU状况、内存、温度、网络状态等信息
        """
        # 如果logger存在，记录开始获取系统快照
        if logger is not None:
            logger.Log(RUNTIME_SNAPSHOT, "开始获取系统快照")
        
        system_snapshot = {}
        
        # CPU信息
        system_snapshot['cpu_percent'] = psutil.cpu_percent(interval=1)
        system_snapshot['cpu_count'] = psutil.cpu_count()
        system_snapshot['cpu_freq'] = psutil.cpu_freq()._asdict() if psutil.cpu_freq() else None
        
        # 内存信息
        memory = psutil.virtual_memory()
        system_snapshot['memory_total'] = memory.total
        system_snapshot['memory_available'] = memory.available
        system_snapshot['memory_used'] = memory.used
        system_snapshot['memory_percent'] = memory.percent
        
        # 温度信息（如果可用）
        try:
            temps = psutil.sensors_temperatures()
            system_snapshot['temperature'] = {k: [t._asdict() for t in v] for k, v in temps.items()} if temps else None
        except (AttributeError, OSError):
            system_snapshot['temperature'] = None
        
        # 网络状态
        network_info = psutil.net_if_addrs()
        system_snapshot['network_interfaces'] = {}
        for interface_name, addrs in network_info.items():
            system_snapshot['network_interfaces'][interface_name] = [
                {
                    'family': str(addr.family),
                    'address': addr.address,
                    'netmask': addr.netmask
                }
                for addr in addrs
            ]
        
        # 网络统计 - 第一次采样
        net_io_1 = psutil.net_io_counters()
        time.sleep(0.1)  # 等待0.1秒
        net_io_2 = psutil.net_io_counters()
        
        # 计算带宽（字节/秒）
        bytes_sent_per_sec = (net_io_2.bytes_sent - net_io_1.bytes_sent) / 0.1
        bytes_recv_per_sec = (net_io_2.bytes_recv - net_io_1.bytes_recv) / 0.1
        
        system_snapshot['network_io'] = {
            'bytes_sent': net_io_2.bytes_sent,
            'bytes_recv': net_io_2.bytes_recv,
            'packets_sent': net_io_2.packets_sent,
            'packets_recv': net_io_2.packets_recv,
            'bytes_sent_per_sec': bytes_sent_per_sec,
            'bytes_recv_per_sec': bytes_recv_per_sec,
            'bandwidth_mbps_sent': (bytes_sent_per_sec * 8) / (1024 * 1024),  # 转换为Mbps
            'bandwidth_mbps_recv': (bytes_recv_per_sec * 8) / (1024 * 1024)   # 转换为Mbps
        }
        
        # 网络延迟测试（ping本地回环）
        try:
            start_time = time.time()
            socket.create_connection(("127.0.0.1", 80), timeout=1).close()
            latency_ms = (time.time() - start_time) * 1000
            system_snapshot['network_latency_ms'] = latency_ms
        except:
            # 如果连接失败，尝试简单的socket创建测试
            try:
                start_time = time.time()
                s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                s.close()
                latency_ms = (time.time() - start_time) * 1000
                system_snapshot['network_latency_ms'] = latency_ms
            except:
                system_snapshot['network_latency_ms'] = None
        
        # GPU信息（如果可用）
        if torch.cuda.is_available():
            system_snapshot['gpu_available'] = True
            system_snapshot['gpu_count'] = torch.cuda.device_count()
            system_snapshot['gpu_devices'] = []
            for i in range(torch.cuda.device_count()):
                gpu_info = {
                    'id': i,
                    'name': torch.cuda.get_device_name(i),
                    'memory_allocated': torch.cuda.memory_allocated(i),
                    'memory_reserved': torch.cuda.memory_reserved(i)
                }
                system_snapshot['gpu_devices'].append(gpu_info)
        else:
            system_snapshot['gpu_available'] = False
            system_snapshot['gpu_count'] = 0
        
        # 如果logger存在，记录系统快照获取成功
        if logger is not None:
            snapshot_summary = f"系统快照获取成功 - CPU: {system_snapshot['cpu_percent']}%, 内存: {system_snapshot['memory_percent']}%, GPU可用: {system_snapshot['gpu_available']}"
            logger.Log(RUNTIME_SNAPSHOT, snapshot_summary)
        
        return system_snapshot
    
    def Load_Model(self, model_path):
        """
        加载torch模型
        
        输入: model_path - 模型文件路径
        输出: bool - 是否成功加载模型
        """
        # 如果logger存在，记录准备加载模型
        if logger is not None:
            logger.Log(RUNTIME_LOAD, f"准备加载模型: {model_path}")
        
        try:
            # 尝试加载torch.jit traced模型
            try:
                self.model = torch.jit.load(model_path)
            except:
                # 如果失败，使用常规torch.load
                self.model = torch.load(model_path, weights_only=False)
            
            # 如果模型有eval方法，设置为评估模式
            if hasattr(self.model, 'eval'):
                self.model.eval()
            
            # 如果GPU可用，将模型移至GPU
            if torch.cuda.is_available():
                self.model = self.model.cuda()
            
            # 如果logger存在，记录加载成功
            if logger is not None:
                logger.Log(RUNTIME_LOAD, f"模型加载成功: {model_path}")
            
            return True
            
        except Exception as e:
            error_msg = f"模型加载失败: {e}"
            print(error_msg)
            
            # 如果logger存在，记录加载失败
            if logger is not None:
                logger.Log(RUNTIME_LOAD, error_msg)
            
            self.model = None
            return False
    
    def Execute(self, model_input):
        """
        执行模型推理
        
        输入: model_input - 模型输入数据
        输出: 模型输出结果
        """
        # 检查模型是否已加载
        if self.model is None:
            error_msg = "模型尚未加载，无法执行推理"
            
            # 如果logger存在，记录错误
            if logger is not None:
                logger.Log(RUNTIME_EXECUTE, error_msg)
            
            raise ValueError(error_msg)
        
        # 如果logger存在，记录开始执行推理
        if logger is not None:
            logger.Log(RUNTIME_EXECUTE, "模型已加载，开始执行推理")
        
        try:
            # 如果GPU可用且输入不在GPU上，将输入移至GPU
            if torch.cuda.is_available() and isinstance(model_input, torch.Tensor):
                if not model_input.is_cuda:
                    model_input = model_input.cuda()
            
            # 执行推理（不计算梯度）
            with torch.no_grad():
                output = self.model(model_input)
            
            # 如果logger存在，记录执行成功
            if logger is not None:
                logger.Log(RUNTIME_EXECUTE, "模型推理执行成功")
            
            return output
            
        except Exception as e:
            error_msg = f"模型推理执行失败: {e}"
            
            # 如果logger存在，记录执行失败及错误信息
            if logger is not None:
                logger.Log(RUNTIME_EXECUTE, error_msg)
            
            raise RuntimeError(error_msg)
